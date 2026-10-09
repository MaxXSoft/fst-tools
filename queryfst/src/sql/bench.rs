//! Opt-in microbenchmarks of the real planner, without FST decoding or CLI startup.

use super::plan::Plan;
use super::value::Cell;
use super::{MatchMode, Options, PeriodicSampling, Report};
use std::collections::{BTreeMap, HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::time::{Duration, Instant};

pub(crate) const PHASES: [&str; 3] = ["compile", "parse_compile", "execute_256"];

pub(crate) fn workloads() -> Vec<(String, String)> {
  let mut cases = vec![
    (
      "simple".into(),
      "SELECT tick, a FROM samples WHERE b = 1".into(),
    ),
    ("aggregate_one".into(), "SELECT SUM(a) FROM samples".into()),
  ];
  for n in [1, 4, 16, 64] {
    let groups = (0..n).map(|i| format!("a + {i}")).collect::<Vec<_>>();
    let projection = groups.iter().rev().cloned().collect::<Vec<_>>().join(", ");
    cases.push((
      format!("groups_{n}"),
      format!(
        "SELECT {projection}, SUM(b) FROM samples GROUP BY {}",
        groups.join(", ")
      ),
    ));
  }
  for n in [4, 64] {
    let projection = (0..n).map(|i| format!("a + {i}")).collect::<Vec<_>>();
    cases.push((
      format!("order_{n}"),
      format!(
        "SELECT {} FROM samples ORDER BY {}",
        projection.join(", "),
        projection
          .iter()
          .rev()
          .cloned()
          .collect::<Vec<_>>()
          .join(", ")
      ),
    ));
  }
  for (name, calls) in [
    (
      "aggregate_unique_64",
      (0..64).map(|i| format!("SUM(a + {i})")).collect::<Vec<_>>(),
    ),
    ("aggregate_repeat_64", vec!["SUM(a + 1)".into(); 64]),
    (
      "aggregate_equivalent_64",
      (0..64)
        .map(|i| ["SUM(known(a))", "sum(is_known((a)))", "SuM(KNOWN(((a))))"][i % 3].into())
        .collect(),
    ),
    (
      "temporal_unique_64",
      (0..64).map(|i| format!("lag(a + {i})")).collect(),
    ),
    ("temporal_repeat_64", vec!["lag(a + 1)".into(); 64]),
    (
      "temporal_equivalent_64",
      (0..64)
        .map(|i| ["lag(a)", "LAG((a))", "LaG(((a)))"][i % 3].into())
        .collect(),
    ),
  ] {
    cases.push((
      name.into(),
      format!("SELECT {} FROM samples", calls.join(", ")),
    ));
  }
  let mut deep = "a".to_string();
  for _ in 0..32 {
    deep = format!("({deep} + 1)");
  }
  cases.push((
    "deep_group_32".into(),
    format!("SELECT {deep}, COUNT(*) FROM samples GROUP BY {deep}"),
  ));
  let mut nested = "a".to_string();
  for _ in 0..12 {
    nested = format!("lag({nested})");
  }
  cases.push((
    "nested_temporal_12".into(),
    format!("SELECT {nested} FROM samples"),
  ));
  cases
}

fn options(sql: String) -> Options {
  Options {
    sql,
    bindings: BTreeMap::new(),
    start: 0,
    end: 255,
    sampling: PeriodicSampling {
      period: 1,
      phase: 0,
    },
    max_callbacks: None,
    max_samples: None,
    max_groups: None,
    max_buffer_rows: None,
    max_duration_ms: None,
    context_before: 0,
    context_after: 0,
    matches: MatchMode::All,
  }
}

/// Includes rows, column names and deterministic completion metadata in the check.
fn execute(mut plan: Plan) -> u64 {
  let mut digest = DefaultHasher::new();
  for column in &plan.columns {
    column.name.hash(&mut digest);
  }
  let mut report = Report::default();
  let mut cells = vec![Cell::Integer(0); 4];
  let mut emit = |_: &[_], row: &[Cell]| {
    row.hash(&mut digest);
    Ok(true)
  };
  for tick in 0..256 {
    cells[0] = Cell::Integer(tick);
    cells[1] = Cell::Integer(tick);
    cells[2] = Cell::Integer(tick % 16);
    cells[3] = Cell::Integer(tick % 2);
    report.sampled_rows += 1;
    if !plan.sample(&cells, &mut report, &mut emit).unwrap() {
      break;
    }
  }
  plan.finish(&mut report, &mut emit).unwrap();
  format!("{report:?}").hash(&mut digest);
  digest.finish()
}

fn measure(mut operation: impl FnMut(), duration: Duration) -> f64 {
  // Calibrate outside the timed interval; amortize clock reads over batches.
  let mut batch = 1;
  loop {
    let start = Instant::now();
    for _ in 0..batch {
      operation();
    }
    if start.elapsed() >= Duration::from_micros(200) {
      break;
    }
    batch *= 2;
  }
  let start = Instant::now();
  let mut iterations = 0_u64;
  while start.elapsed() < duration {
    for _ in 0..batch {
      operation();
    }
    iterations += batch;
  }
  start.elapsed().as_nanos() as f64 / iterations as f64
}

#[test]
#[ignore = "raw timing worker; prefer cargo bench -p queryfst --bench sql_compile"]
fn sql_compile_microbench() {
  let ms = std::env::var("SQL_BENCH_MS")
    .unwrap_or("30".into())
    .parse()
    .unwrap();
  let rotation: usize = std::env::var("SQL_BENCH_ROTATION")
    .unwrap_or("0".into())
    .parse()
    .unwrap();
  for measurement in run(ms, rotation) {
    println!("BENCH {measurement}");
  }
}

/// One timing round; the Cargo benchmark runner handles processes and statistics.
pub(crate) fn run(ms: u64, rotation: usize) -> Vec<serde_json::Value> {
  assert!(ms > 0, "measurement duration must be positive");
  let duration = Duration::from_millis(ms);
  let mut measurements = Vec::new();
  let schema = ["tick", "sample_index", "a", "b"]
    .into_iter()
    .enumerate()
    .map(|(i, name)| (name.to_string(), (i, 8)))
    .collect::<HashMap<_, _>>();
  let mut cases = workloads();
  let count = cases.len();
  cases.rotate_left(rotation % count);
  for (name, sql) in cases {
    let options = options(sql);
    let ast = Plan::parse(&options.sql).unwrap();
    let fingerprint = execute(Plan::compile(&ast, &schema, &options).unwrap());
    for phase in PHASES {
      let ns = measure(
        || match phase {
          "compile" => {
            black_box(Plan::compile(black_box(&ast), &schema, &options).unwrap());
          }
          "parse_compile" => {
            let ast = Plan::parse(black_box(&options.sql)).unwrap();
            black_box(Plan::compile(&ast, &schema, &options).unwrap());
          }
          _ => {
            black_box(execute(Plan::compile(&ast, &schema, &options).unwrap()));
          }
        },
        duration,
      );
      measurements.push(
        serde_json::json!({"case":name, "phase":phase, "ns":ns, "fingerprint":fingerprint.to_string()})
      );
    }
  }
  measurements
}
