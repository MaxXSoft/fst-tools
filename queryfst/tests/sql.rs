use fstapi::{Writer, var_dir, var_type};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Generates deterministic fixtures under Cargo's ignored test directory.
fn directory() -> TempDir {
  tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap()
}

fn fixture(dir: &Path) {
  let mut writer = Writer::create(dir.join("sample.fst"), true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 8, "a", None)
    .unwrap();
  writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 8, "alias", Some(a))
    .unwrap();
  let b = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "b", None)
    .unwrap();
  let valid = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "valid", None)
    .unwrap();
  let unknown = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 4, "unknown", None)
    .unwrap();
  for tick in 0..5 {
    writer.emit_time_change(tick).unwrap();
    writer
      .emit_value_change(a, format!("{tick:08b}").as_bytes())
      .unwrap();
    writer
      .emit_value_change(b, if tick % 2 == 0 { b"0" } else { b"1" })
      .unwrap();
    writer
      .emit_value_change(valid, if (2..4).contains(&tick) { b"1" } else { b"0" })
      .unwrap();
    if tick == 0 {
      writer.emit_value_change(unknown, b"01xz").unwrap();
    }
  }
  writer.emit_time_change(5).unwrap();
  drop(writer);
  std::fs::write(
    dir.join("bindings.json"),
    serde_json::to_vec(
      &json!({"a":"a", "alias":"alias", "b":"b", "valid":"valid", "unknown":"unknown"}),
    )
    .unwrap(),
  )
  .unwrap();
}

fn invoke(dir: &Path, sql: &str, extra: &[&str]) -> Output {
  Command::new(env!("CARGO_BIN_EXE_queryfst"))
    .arg(dir.join("sample.fst"))
    .args(["--format", "jsonl", "--sql", sql, "--bindings"])
    .arg(dir.join("bindings.json"))
    .args(["--period", "1", "--start", "0", "--end", "4"])
    .args(extra)
    .output()
    .unwrap()
}

fn run(dir: &Path, sql: &str, extra: &[&str]) -> Vec<Value> {
  let output = invoke(dir, sql, extra);
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  String::from_utf8(output.stdout)
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect()
}

fn rows(records: &[Value]) -> Vec<Value> {
  records
    .iter()
    .filter(|row| row["type"] == "row")
    .map(|row| row["values"].clone())
    .collect()
}

#[test]
fn complete_timestamp_alias_sampling_grouping_and_empty_aggregates() {
  let dir = directory();
  fixture(dir.path());
  let records = run(
    dir.path(),
    "SELECT b, COUNT(*) AS n, SUM(a) AS total, MIN(a), MAX(a), SUM(a-alias) FROM samples GROUP BY b ORDER BY b",
    &[],
  );
  assert_eq!(
    rows(&records),
    vec![
      json!(["0", "3", "6", "0", "4", "0"]),
      json!(["1", "2", "4", "1", "3", "0"])
    ]
  );
  let records = run(
    dir.path(),
    "SELECT tick FROM samples WHERE a=1 AND b=0",
    &[],
  );
  assert!(
    rows(&records).is_empty(),
    "must not see a half-updated timestamp"
  );
  assert_eq!(
    rows(&run(
      dir.path(),
      "SELECT COUNT(*), SUM(a), MIN(a) FROM samples WHERE a=99",
      &[]
    )),
    vec![json!(["0", null, null])]
  );
  assert!(
    rows(&run(
      dir.path(),
      "SELECT b, COUNT(*) FROM samples WHERE a=99 GROUP BY b",
      &[]
    ))
    .is_empty()
  );
}

#[test]
fn temporal_functions_advance_before_where_and_share_identical_state() {
  let dir = directory();
  fixture(dir.path());
  let records = run(
    dir.path(),
    "SELECT tick, lag(a), hold(a,b=1), run_length(valid), runs(valid), changed(a) FROM samples WHERE b=0 ORDER BY tick",
    &[],
  );
  assert_eq!(
    rows(&records),
    vec![
      json!(["0", null, null, "0", false, true]),
      json!(["2", "1", "1", "1", true, true]),
      json!(["4", "3", "3", "0", false, true])
    ]
  );
  let records = run(
    dir.path(),
    "SELECT tick, run_length(valid), run_length(valid) FROM samples WHERE run_length(valid)=2",
    &[],
  );
  assert_eq!(rows(&records), vec![json!(["3", "2", "2"])]);
}

#[test]
fn exact_bits_unknown_truth_and_lazy_case_are_explicit() {
  let dir = directory();
  fixture(dir.path());
  let records = run(
    dir.path(),
    "SELECT raw(a), raw(unknown), known(unknown), unknown+1, CASE WHEN a=0 THEN 9 ELSE 1/a END, NULL IS NOT TRUE, 1 IS TRUE, NULL IS FALSE, COALESCE(1,1/0) FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(
    rows(&records),
    vec![json!([
      "00000000", "01xz", false, null, "9", true, true, false, "1"
    ])]
  );
  let records = run(
    dir.path(),
    "SELECT SUM(a) IS NULL, SUM(a) BETWEEN 9 AND 11, COUNT(*) IN (4,5) FROM samples",
    &[],
  );
  assert_eq!(rows(&records), vec![json!([false, true, true])]);
}

#[test]
fn streaming_first_and_bounded_last_do_not_need_an_unbounded_sort() {
  let dir = directory();
  fixture(dir.path());
  let first = run(
    dir.path(),
    "SELECT tick FROM samples ORDER BY tick LIMIT 1",
    &["--max-buffer-rows", "1"],
  );
  assert_eq!(rows(&first), vec![json!(["0"])]);
  assert_eq!(first.last().unwrap()["complete"], true);
  let last = run(
    dir.path(),
    "SELECT tick FROM samples ORDER BY tick DESC LIMIT 1",
    &["--max-buffer-rows", "1"],
  );
  assert_eq!(rows(&last), vec![json!(["4"])]);
  assert_eq!(last.last().unwrap()["complete"], true);
}

#[test]
fn callbacks_samples_groups_and_buffers_stop_with_partial_status() {
  let dir = directory();
  fixture(dir.path());
  let callbacks = run(
    dir.path(),
    "SELECT COUNT(*) FROM samples",
    &["--max-callbacks", "1"],
  );
  assert_eq!(rows(&callbacks), vec![json!(["0"])]);
  assert_eq!(callbacks.last().unwrap()["decoded_callbacks"], "1");
  assert_eq!(callbacks.last().unwrap()["complete"], false);
  let samples = run(
    dir.path(),
    "SELECT COUNT(*) FROM samples",
    &["--max-samples", "2"],
  );
  assert_eq!(rows(&samples), vec![json!(["2"])]);
  assert_eq!(samples.last().unwrap()["complete"], false);
  let groups = run(
    dir.path(),
    "SELECT b, COUNT(*) FROM samples GROUP BY b",
    &["--max-groups", "1"],
  );
  assert_eq!(groups.last().unwrap()["complete"], false);
  let buffer = run(
    dir.path(),
    "SELECT a FROM samples ORDER BY b",
    &["--max-buffer-rows", "1"],
  );
  assert_eq!(buffer.last().unwrap()["complete"], false);
}

#[test]
fn context_windows_include_before_after_and_expose_match_flags() {
  let dir = directory();
  fixture(dir.path());
  let records = run(
    dir.path(),
    "SELECT tick FROM samples WHERE a=2 ORDER BY tick",
    &["--before", "1", "--after", "1"],
  );
  assert_eq!(
    rows(&records),
    vec![json!(["1", false]), json!(["2", true]), json!(["3", false])]
  );
}

#[test]
fn unsupported_constructs_fail_even_when_where_would_hide_them() {
  let dir = directory();
  fixture(dir.path());
  for sql in [
    "SELECT a FROM samples WHERE a=99 UNION SELECT a FROM samples",
    "SELECT a FROM samples JOIN samples s ON true",
    "SELECT a FROM samples GROUP BY b",
    "SELECT SUM(COUNT(*)) FROM samples",
    "SELECT a FROM samples WHERE SUM(a)>0",
    "SELECT a ~ 'x' FROM samples WHERE a=99",
    "SELECT timeouts(valid,b,a) FROM samples",
  ] {
    let output = invoke(dir.path(), sql, &[]);
    assert!(!output.status.success(), "unexpected success: {sql}");
  }
}

#[test]
fn no_binding_queries_synthesize_periodic_samples_without_decoding() {
  let dir = directory();
  fixture(dir.path());
  std::fs::write(
    dir.path().join("bindings.json"),
    serde_json::to_vec(&BTreeMap::<String, String>::new()).unwrap(),
  )
  .unwrap();
  let records = run(
    dir.path(),
    "SELECT COUNT(*), MIN(tick), MAX(tick) FROM samples",
    &[],
  );
  assert_eq!(rows(&records), vec![json!(["5", "0", "4"])]);
  assert_eq!(records.last().unwrap()["decoded_callbacks"], "0");
}

#[test]
fn selected_first_and_last_matches_keep_their_entire_context_windows() {
  let dir = directory();
  fixture(dir.path());
  let sql = "SELECT tick FROM samples WHERE valid=1 ORDER BY tick";
  let first = run(
    dir.path(),
    sql,
    &["--matches", "first", "--before", "1", "--after", "1"],
  );
  assert_eq!(
    rows(&first),
    vec![json!(["1", false]), json!(["2", true]), json!(["3", false])]
  );
  assert_eq!(first.last().unwrap()["complete"], true);
  let last = run(
    dir.path(),
    sql,
    &["--matches", "last", "--before", "1", "--after", "1"],
  );
  assert_eq!(
    rows(&last),
    vec![json!(["2", false]), json!(["3", true]), json!(["4", false])]
  );
  let invalid = invoke(
    dir.path(),
    "SELECT tick FROM samples WHERE valid=1 LIMIT 1",
    &["--before", "1"],
  );
  assert!(
    !invalid.status.success(),
    "LIMIT would select a preceding row rather than a matching trigger"
  );
}

#[test]
fn request_deadlines_distinguish_expiration_from_unresolved_eof() {
  let dir = directory();
  fixture(dir.path());
  let matched = run(
    dir.path(),
    "SELECT SUM(timeouts(tick=0,tick=2,2)) FROM samples",
    &[],
  );
  assert_eq!(rows(&matched), vec![json!(["0"])]);
  let expired = run(
    dir.path(),
    "SELECT SUM(timeouts(tick=0,0,2)) FROM samples",
    &[],
  );
  assert_eq!(rows(&expired), vec![json!(["1"])]);
  let pending = run(
    dir.path(),
    "SELECT SUM(timeouts(tick=4,0,2)) FROM samples",
    &[],
  );
  assert_eq!(rows(&pending), vec![json!(["0"])]);
  assert_eq!(pending.last().unwrap()["pending_requests"], "1");
}

#[test]
fn grouping_and_ordering_normalize_sql_equivalent_numeric_types() {
  let dir = directory();
  fixture(dir.path());
  let grouped = run(
    dir.path(),
    "SELECT CASE WHEN b=0 THEN true ELSE 1 END AS k,COUNT(*) FROM samples GROUP BY CASE WHEN b=0 THEN true ELSE 1 END",
    &[],
  );
  assert_eq!(rows(&grouped), vec![json!(["1", "5"])]);
  let ordered = run(
    dir.path(),
    "SELECT tick FROM samples ORDER BY CASE WHEN tick=0 THEN true ELSE 0 END,tick",
    &[],
  );
  assert_eq!(
    rows(&ordered),
    vec![
      json!(["1"]),
      json!(["2"]),
      json!(["3"]),
      json!(["4"]),
      json!(["0"])
    ]
  );
}

#[test]
fn scalar_dispatch_preserves_aliases_nulls_and_lazy_branches() {
  let dir = directory();
  fixture(dir.path());
  let records = run(
    dir.path(),
    "SELECT CoAlEsCe(NULL, AbS(-3), 1/0), KnOwN(unknown), Is_KnOwN(a),
       HeX(255), BiT(unknown,2), bit(unknown,0), bit(a,NULL), +a, -a, NOT NULL,
       CASE a WHEN 0 THEN 7 ELSE 1/0 END,
       CASE NULL WHEN NULL THEN 1/0 ELSE 8 END,
       a NOT BETWEEN 1 AND 3, a NOT IN (1,2)
     FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(
    rows(&records),
    vec![json!([
      "3", false, true, "ff", "1", null, null, "0", "0", null, "7", "8", true, true
    ])]
  );
  let records = run(
    dir.path(),
    "SELECT SUM((a >> 1) & 1), SUM(bit(a,1)), ABS(SUM(a)), COUNT(*)
     FROM samples ORDER BY COALESCE(MAX(a),0)",
    &[],
  );
  assert_eq!(rows(&records), vec![json!(["2", "2", "10", "5"])]);
}

#[test]
fn nested_temporal_dependencies_and_unknown_resets_survive_filtering() {
  let dir = directory();
  fixture(dir.path());
  let records = run(
    dir.path(),
    "SELECT tick, lag(lag(a)), lag(a), changed(lag(a)),
       hold(a, CASE WHEN tick=2 THEN NULL WHEN tick=1 THEN TRUE ELSE FALSE END),
       run_length(CASE WHEN tick=2 THEN NULL ELSE TRUE END),
       runs(CASE WHEN tick=2 THEN NULL ELSE TRUE END)
     FROM samples WHERE tick IN (0,2,3,4)",
    &[],
  );
  assert_eq!(
    rows(&records),
    vec![
      json!(["0", null, null, false, null, "1", true]),
      json!(["2", "0", "1", true, null, null, null]),
      json!(["3", "1", "2", true, null, "1", true]),
      json!(["4", "2", "3", true, null, "2", false]),
    ]
  );
}

#[test]
fn function_arity_and_context_errors_are_rejected_before_sampling() {
  let dir = directory();
  fixture(dir.path());
  for expression in [
    "coalesce()",
    "known()",
    "is_known(a,b)",
    "abs()",
    "hex(a,b)",
    "bit(a)",
    "raw(a,b)",
    "lag()",
    "changed(a,b)",
    "hold(a)",
    "run_length()",
    "runs(a,b)",
    "timeouts(a,b)",
    "timeouts(a,b,1,a,b)",
    "timeouts(a,b,NULL)",
    "unknown_function(a)",
    "SUM(*)",
    "MIN(*)",
    "MAX(*)",
    "COUNT(a,b)",
    "bit(*)",
    "lag(SUM(a))",
    "COALESCE(SUM(COUNT(*)),0)",
  ] {
    let sql = format!("SELECT {expression} FROM samples WHERE FALSE");
    assert!(
      !invoke(dir.path(), &sql, &[]).status.success(),
      "unexpected success: {sql}"
    );
  }
  for sql in [
    "SELECT a FROM samples GROUP BY b ORDER BY abs(a)",
    "SELECT lag(a) FROM samples GROUP BY b",
    "SELECT a FROM samples WHERE coalesce(SUM(a),0)=0",
  ] {
    assert!(
      !invoke(dir.path(), sql, &[]).status.success(),
      "unexpected success: {sql}"
    );
  }
}

#[test]
fn sort_direction_and_null_placement_are_independent() {
  let dir = directory();
  fixture(dir.path());
  for (ordering, expected) in [
    ("ASC NULLS FIRST", [2, 4, 0, 3, 1]),
    ("ASC NULLS LAST", [4, 0, 3, 1, 2]),
    ("DESC NULLS FIRST", [2, 3, 1, 4, 0]),
    ("DESC NULLS LAST", [3, 1, 4, 0, 2]),
  ] {
    let sql = format!(
      "SELECT tick, CASE WHEN tick=2 THEN NULL ELSE tick%2 END AS k
       FROM samples ORDER BY k {ordering}, tick DESC"
    );
    let records = run(dir.path(), &sql, &[]);
    let ticks: Vec<_> = rows(&records).iter().map(|row| row[0].clone()).collect();
    assert_eq!(
      ticks,
      expected.map(|tick| json!(tick.to_string())),
      "{ordering}"
    );
  }
}

#[test]
fn final_output_truncation_preserves_the_sampling_stop_reason() {
  let dir = directory();
  fixture(dir.path());
  for sql in [
    "SELECT b, COUNT(*) FROM samples GROUP BY b",
    "SELECT b, COUNT(*) FROM samples GROUP BY b ORDER BY 2 DESC",
    "SELECT tick FROM samples ORDER BY b,tick",
  ] {
    let records = run(dir.path(), sql, &["--max-samples", "3", "--limit", "1"]);
    assert_eq!(rows(&records).len(), 1, "{sql}");
    let summary = records.last().unwrap();
    assert_eq!(summary["complete"], false, "{sql}");
    assert_eq!(summary["aggregate_final"], false, "{sql}");
    assert_eq!(summary["reason"], "sample_budget", "{sql}");
    assert_eq!(summary["output_reason"], "row_budget_exhausted", "{sql}");
    assert_eq!(summary["output_truncated"], true, "{sql}");
  }
}
