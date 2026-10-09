//! Serial, repeatable comparisons of saved SQL benchmark executables.

use crate::sql::bench;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Measure SQL planning and synthetic execution, excluding FST decoding and I/O.
/// Without --compare, benchmarks this executable. Writes JSON to stdout.
#[derive(Parser)]
struct Options {
  /// Number of serial rounds, rotating executable and workload order.
  #[arg(long, default_value_t = 9, value_parser = clap::value_parser!(u32).range(1..))]
  rounds: u32,
  /// Minimum measurement time per workload and phase, in milliseconds.
  #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
  ms: u64,
  /// Saved cargo-bench executable; repeat to compare versions.
  #[arg(long, value_name = "LABEL=PATH")]
  compare: Vec<String>,
  /// Saved release test executable from the previous Python-based harness.
  #[arg(long, value_name = "LABEL=PATH")]
  legacy_compare: Vec<String>,
  #[arg(long, hide = true)]
  worker: bool,
  #[arg(long, hide = true, default_value_t = 0)]
  rotation: usize,
  // Cargo passes this argument even with harness = false.
  #[arg(long, hide = true)]
  bench: bool,
}

#[derive(Serialize)]
struct Artifact {
  path: PathBuf,
  sha256: String,
  legacy: bool,
}

#[derive(Deserialize, Serialize)]
struct Measurement {
  case: String,
  phase: String,
  ns: f64,
  fingerprint: String,
}

#[derive(Serialize)]
struct Record {
  variant: String,
  round: u32,
  #[serde(flatten)]
  measurement: Measurement,
}

pub(super) fn run() -> Result<()> {
  let options = Options::parse();
  if options.worker {
    for measurement in bench::run(options.ms, options.rotation) {
      println!("BENCH {measurement}");
    }
    return Ok(());
  }
  let artifacts = artifacts(&options)?;
  let expected = bench::workloads()
    .into_iter()
    .flat_map(|(case, _)| bench::PHASES.map(|phase| (case.clone(), phase.to_string())))
    .collect::<BTreeSet<_>>();
  let mut fingerprints = BTreeMap::new();
  let mut records = Vec::new();
  for round in 0..options.rounds {
    let mut order = artifacts.iter().collect::<Vec<_>>();
    let count = order.len();
    order.rotate_left(round as usize % count);
    if (round as usize / count) % 2 == 1 {
      order.reverse();
    }
    for (label, artifact) in order {
      eprintln!(
        "SQL benchmark: round {}/{}, {label}",
        round + 1,
        options.rounds
      );
      let measurements = measure(artifact, options.ms, round as usize)?;
      let mut found = BTreeSet::new();
      for measurement in measurements {
        let key = (measurement.case.clone(), measurement.phase.clone());
        if !expected.contains(&key) || !found.insert(key.clone()) {
          return Err(format!("unexpected or duplicate result: {label} {key:?}").into());
        }
        if !measurement.ns.is_finite() || measurement.ns <= 0.0 {
          return Err(format!("invalid timing: {label} {key:?}").into());
        }
        let previous = fingerprints
          .entry(key.clone())
          .or_insert_with(|| measurement.fingerprint.clone());
        if *previous != measurement.fingerprint {
          return Err(format!("result mismatch: {label} {key:?}").into());
        }
        records.push(Record {
          variant: label.clone(),
          round,
          measurement,
        });
      }
      if found != expected {
        return Err(
          format!(
            "missing results: {label}: {:?}",
            expected.difference(&found).collect::<Vec<_>>()
          )
          .into(),
        );
      }
    }
  }
  let summary = summarize(&records, &artifacts, &expected);
  println!(
    "{}",
    serde_json::to_string_pretty(&json!({
      "platform": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
      "artifacts": artifacts,
      "rounds": options.rounds,
      "ms": options.ms,
      "records": records,
      "summary": summary,
    }))?
  );
  Ok(())
}

fn artifacts(options: &Options) -> Result<BTreeMap<String, Artifact>> {
  let mut paths = Vec::new();
  for (items, legacy) in [(&options.compare, false), (&options.legacy_compare, true)] {
    for item in items {
      let (label, path) = item.split_once('=').ok_or("expected LABEL=PATH")?;
      if label.is_empty() || path.is_empty() {
        return Err("label and path must be nonempty".into());
      }
      paths.push((label.to_string(), PathBuf::from(path), legacy));
    }
  }
  if paths.is_empty() {
    paths.push(("current".into(), std::env::current_exe()?, false));
  }
  let mut artifacts = BTreeMap::new();
  for (label, path, legacy) in paths {
    let path = path.canonicalize()?;
    let sha256 = format!("{:x}", Sha256::digest(std::fs::read(&path)?));
    if artifacts
      .insert(
        label.clone(),
        Artifact {
          path,
          sha256,
          legacy,
        },
      )
      .is_some()
    {
      return Err(format!("duplicate label: {label}").into());
    }
  }
  Ok(artifacts)
}

fn measure(artifact: &Artifact, ms: u64, rotation: usize) -> Result<Vec<Measurement>> {
  let mut command = Command::new(&artifact.path);
  if artifact.legacy {
    command
      .args([
        "sql::bench::sql_compile_microbench",
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
      ])
      .env("SQL_BENCH_MS", ms.to_string())
      .env("SQL_BENCH_ROTATION", rotation.to_string());
  } else {
    command.args([
      "--worker",
      "--ms",
      &ms.to_string(),
      "--rotation",
      &rotation.to_string(),
    ]);
  }
  let output = command.output()?;
  if !output.status.success() {
    return Err(
      format!(
        "{} failed ({}): {}",
        artifact.path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
      )
      .into(),
    );
  }
  String::from_utf8(output.stdout)?
    .lines()
    // libtest can print its test-name prefix before the first measurement.
    .filter_map(|line| line.split_once("BENCH ").map(|(_, record)| record))
    .map(|line| serde_json::from_str(line).map_err(Into::into))
    .collect()
}

fn summarize(
  records: &[Record],
  artifacts: &BTreeMap<String, Artifact>,
  expected: &BTreeSet<(String, String)>,
) -> Vec<serde_json::Value> {
  let mut summary = Vec::new();
  for label in artifacts.keys() {
    for (case, phase) in expected {
      let mut values = records
        .iter()
        .filter(|record| {
          record.variant == *label
            && record.measurement.case == *case
            && record.measurement.phase == *phase
        })
        .map(|record| record.measurement.ns)
        .collect::<Vec<_>>();
      let median = median(&mut values);
      let mut deviations = values
        .iter()
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
      summary.push(json!({
        "variant": label, "case": case, "phase": phase, "median_ns": median,
        "mad_ns": self::median(&mut deviations),
        "min_ns": values[0], "max_ns": values[values.len() - 1],
      }));
    }
  }
  summary
}

fn median(values: &mut [f64]) -> f64 {
  values.sort_by(f64::total_cmp);
  let mid = values.len() / 2;
  if values.len().is_multiple_of(2) {
    (values[mid - 1] + values[mid]) / 2.0
  } else {
    values[mid]
  }
}
