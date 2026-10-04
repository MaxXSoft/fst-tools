//! Independent SQL contract regressions discovered during implementation review.
use fstapi::{Writer, var_dir, var_type};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

fn fixture(path: &Path) {
  let mut writer = Writer::create(path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 8, "a", None)
    .unwrap();
  for (time, value) in [(0, b"00000101"), (4, b"00000110"), (8, b"00000111")] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(a, value).unwrap();
  }
  writer.emit_time_change(10).unwrap();
}

fn dir() -> tempfile::TempDir {
  tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap()
}

fn invoke(path: &Path, sql: &str, extra: &[&str]) -> Output {
  Command::new(env!("CARGO_BIN_EXE_queryfst"))
    .arg(path)
    .args(["--json", "--bind", "a=a", "--period", "1", "--sql", sql])
    .args(extra)
    .output()
    .unwrap()
}

fn rows(output: &Output) -> Vec<Value> {
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  String::from_utf8_lossy(&output.stdout)
    .lines()
    .map(|line| serde_json::from_str::<Value>(line).unwrap())
    .filter(|record| record["type"] == "row")
    .map(|record| record["values"].clone())
    .collect()
}

#[test]
fn rejects_ignored_table_modifiers_and_invalid_order_ordinals() {
  let dir = dir();
  let path = dir.path().join("modifiers.fst");
  fixture(&path);
  for sql in [
    "SELECT tick FROM samples WITH ORDINALITY LIMIT 1",
    "SELECT tick FROM samples ORDER BY 0 LIMIT 1",
    "SELECT tick FROM samples ORDER BY 2 LIMIT 1",
  ] {
    let output = invoke(&path, sql, &[]);
    assert!(
      !output.status.success(),
      "silently accepted {sql}: {}",
      String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stdout.is_empty(), "{sql}");
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["type"], "error");
  }
}

#[test]
fn coalesce_evaluates_only_through_the_first_nonnull_argument() {
  let dir = dir();
  let path = dir.path().join("coalesce.fst");
  fixture(&path);
  let output = invoke(
    &path,
    "SELECT coalesce(1, 1/0), coalesce(NULL, 2, 1/0) FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(rows(&output), vec![json!(["1", "2"])]);
}

#[test]
fn null_text_comparisons_propagate_null_and_distinct_uses_equality_coercion() {
  let dir = dir();
  let path = dir.path().join("comparison.fst");
  fixture(&path);
  let output = invoke(
    &path,
    "SELECT NULL = 'x', 'x' = NULL, NULL || 'x' FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(rows(&output), vec![json!([null, null, null])]);
  let output = invoke(
    &path,
    "SELECT 1 IS DISTINCT FROM true, 1 = true, NULL IS NOT DISTINCT FROM NULL FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(rows(&output), vec![json!([false, true, true])]);
}

#[test]
fn raw_qualified_binding_preserves_the_declared_width() {
  let dir = dir();
  let path = dir.path().join("qualified.fst");
  fixture(&path);
  let output = invoke(
    &path,
    "SELECT raw(a), raw(samples.a) FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(rows(&output), vec![json!(["00000101", "00000101"])]);
}

#[test]
fn an_observation_inside_a_prior_recording_gap_cannot_establish_current_state() {
  let dir = dir();
  let path = dir.path().join("inside-gap.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "a", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(a, b"1").unwrap();
  writer.emit_time_change(5).unwrap();
  writer.emit_dump_active(false).unwrap();
  // libfst stores this callback, but later changes before recording resumes
  // can still be absent. Its value cannot establish state at time 12.
  writer.emit_time_change(7).unwrap();
  writer.emit_value_change(a, b"0").unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_dump_active(true).unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);
  let output = invoke(
    &path,
    "SELECT known(a), a = 0 FROM samples LIMIT 1",
    &["--start", "12", "--end", "15"],
  );
  assert_eq!(rows(&output), vec![json!([false, null])]);
}

#[test]
fn a_same_sample_response_frees_capacity_for_the_next_request() {
  let dir = dir();
  let path = dir.path().join("deadline-capacity.fst");
  fixture(&path);
  let output = invoke(
    &path,
    "SELECT tick, timeouts(1, tick = 1, 5) FROM samples LIMIT 2",
    &["--max-buffer-rows", "1"],
  );
  assert_eq!(rows(&output), vec![json!(["0", "0"]), json!(["1", "0"])]);
  let summary: Value = serde_json::from_str(
    String::from_utf8_lossy(&output.stdout)
      .lines()
      .last()
      .unwrap(),
  )
  .unwrap();
  assert_eq!(summary["complete"], true);
  assert_eq!(summary["pending_requests"], "1");
}

#[test]
fn distinct_agrees_with_equality_for_known_wide_logic_that_fits_an_integer() {
  let dir = dir();
  let path = dir.path().join("wide-comparison.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 128, "a", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(a, &[b'0'; 128]).unwrap();
  writer.emit_time_change(2).unwrap();
  drop(writer);
  let output = invoke(
    &path,
    "SELECT a = 0, a IS DISTINCT FROM 0, a IS NOT DISTINCT FROM 0 FROM samples LIMIT 1",
    &[],
  );
  assert_eq!(rows(&output), vec![json!([true, false, true])]);
}

#[test]
fn recording_can_resume_at_the_first_sample_timestamp() {
  let dir = dir();
  let path = dir.path().join("resume-at-start.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "a", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(a, b"1").unwrap();
  writer.emit_time_change(5).unwrap();
  writer.emit_dump_active(false).unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_dump_active(true).unwrap();
  writer.emit_value_change(a, b"0").unwrap();
  writer.emit_time_change(12).unwrap();
  drop(writer);
  let output = invoke(
    &path,
    "SELECT tick, a FROM samples LIMIT 1",
    &["--start", "10"],
  );
  assert_eq!(rows(&output), vec![json!(["10", "0"])]);
}

#[test]
fn callback_budget_does_not_publish_partially_updated_timestamp_state() {
  let dir = dir();
  let path = dir.path().join("partial-timestamp.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "a", None)
    .unwrap();
  let b = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "b", None)
    .unwrap();
  for (time, value) in [(0, b"0"), (1, b"1")] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(a, value).unwrap();
    writer.emit_value_change(b, value).unwrap();
  }
  writer.emit_time_change(2).unwrap();
  drop(writer);
  let output = invoke(
    &path,
    "SELECT tick, a, b FROM samples",
    &["--bind", "b=b", "--max-callbacks", "3"],
  );
  assert_eq!(rows(&output), vec![json!(["0", "0", "0"])]);
  let summary: Value = serde_json::from_str(
    String::from_utf8_lossy(&output.stdout)
      .lines()
      .last()
      .unwrap(),
  )
  .unwrap();
  assert_eq!(summary["complete"], false);
  assert_eq!(summary["sampled_rows"], "1");
  assert_eq!(summary["processed_through"], "0");
}
