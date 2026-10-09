//! Opt-in microbenchmarks of the real planner, without FST decoding or CLI startup.

use super::plan::Plan;
use super::value::Cell;
use super::{MatchMode, Options, PeriodicSampling, Report};
use criterion::{BenchmarkId, Criterion};
use std::collections::{BTreeMap, HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::hint::black_box;

fn workloads() -> Vec<(String, String)> {
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

fn schema() -> HashMap<String, (usize, u32)> {
  ["tick", "sample_index", "a", "b"]
    .into_iter()
    .enumerate()
    .map(|(i, name)| (name.to_string(), (i, 8)))
    .collect()
}

/// Registers the same workloads and timing boundaries for Criterion.
// Called by the separate benchmark target, not the binary's unit-test harness.
#[allow(dead_code)]
pub(crate) fn benchmarks(criterion: &mut Criterion) {
  let schema = schema();
  let mut group = criterion.benchmark_group("sql_compile");
  for (name, sql) in workloads() {
    let options = options(sql);
    let ast = Plan::parse(&options.sql).unwrap();
    let fingerprint = execute(Plan::compile(&ast, &schema, &options).unwrap());

    group.bench_function(BenchmarkId::new("compile", &name), |b| {
      b.iter(|| {
        // Keep destruction inside the measurement, as in the original harness.
        black_box(Plan::compile(black_box(&ast), &schema, &options).unwrap());
      });
    });
    group.bench_function(BenchmarkId::new("parse_compile", &name), |b| {
      b.iter(|| {
        let ast = Plan::parse(black_box(&options.sql)).unwrap();
        black_box(Plan::compile(&ast, &schema, &options).unwrap());
      });
    });
    group.bench_function(BenchmarkId::new("execute_256", &name), |b| {
      b.iter(|| {
        // This phase includes planning, execution and output fingerprinting.
        black_box(execute(Plan::compile(&ast, &schema, &options).unwrap()));
      });
    });

    // Check repeatability outside Criterion's measured closures.
    assert_eq!(
      execute(Plan::compile(&ast, &schema, &options).unwrap()),
      fingerprint,
      "non-repeatable results for {name}",
    );
  }
  group.finish();
}

/// Export deterministic row/schema/report digests separately from timing data.
/// Compare this line across revisions built with the same toolchain and target.
#[test]
fn workload_fingerprints() {
  let schema = schema();
  let fingerprints = workloads()
    .into_iter()
    .map(|(name, sql)| {
      let options = options(sql);
      let ast = Plan::parse(&options.sql).unwrap();
      let fingerprint = execute(Plan::compile(&ast, &schema, &options).unwrap());
      (name, fingerprint.to_string())
    })
    .collect::<BTreeMap<_, _>>();
  println!("SQL_FINGERPRINTS {}", serde_json::json!(fingerprints));
}
