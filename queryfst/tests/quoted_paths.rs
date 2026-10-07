use fstapi::{Writer, scope_type, var_dir, var_type};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

fn test_dir() -> tempfile::TempDir {
  tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap()
}

fn fixture(path: &Path) {
  let mut writer = Writer::create(path, true).unwrap();
  writer.set_scope(scope_type::VCD_MODULE, "top", "").unwrap();
  let bus = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 8, "bus [7:0]", None)
    .unwrap();
  writer
    .create_var(
      var_type::VCD_WIRE,
      var_dir::OUTPUT,
      8,
      "bus_alias",
      Some(bus),
    )
    .unwrap();
  let quoted = writer
    .create_var(
      var_type::VCD_WIRE,
      var_dir::OUTPUT,
      4,
      "quote\"λ\\name",
      None,
    )
    .unwrap();
  let other = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 8, "other", None)
    .unwrap();
  let ignored = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "unused", None)
    .unwrap();
  writer.set_upscope();
  for (time, value) in [(0, b"00000101"), (4, b"00000110"), (8, b"00000111")] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(bus, value).unwrap();
    writer.emit_value_change(quoted, b"00XZ").unwrap();
    writer.emit_value_change(other, b"00001001").unwrap();
    writer
      .emit_var_len_value_change(ignored, b"not selected")
      .unwrap();
  }
  writer.emit_time_change(10).unwrap();
}

fn invoke(path: &Path, sql: &str, extra: &[&str]) -> Output {
  Command::new(env!("CARGO_BIN_EXE_queryfst"))
    .arg(path)
    .args(["--json", "--period", "4", "--sql", sql])
    .args(extra)
    .output()
    .unwrap()
}

fn records(output: Output) -> Vec<Value> {
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
    .filter(|r| r["type"] == "row")
    .map(|r| r["values"].clone())
    .collect()
}

#[test]
fn quoted_paths_support_qualified_aliases_filters_raw_width_and_selected_masks() {
  let dir = test_dir();
  let path = dir.path().join("quoted.fst");
  fixture(&path);
  let sql = r#"SELECT tick, raw("top.bus [7:0]"), samples."top.bus_alias"
               FROM samples WHERE "top.bus [7:0]" >= 6 ORDER BY tick"#;
  let result = records(invoke(&path, sql, &[]));
  assert_eq!(
    rows(&result),
    vec![json!(["4", "00000110", "6"]), json!(["8", "00000111", "7"])]
  );
  assert_eq!(result[0]["sql"], sql);
  assert_eq!(result.last().unwrap()["decoded_callbacks"], "3");
  assert_eq!(result.last().unwrap()["complete"], true);
}

#[test]
fn sql_escaping_and_nested_functions_preserve_unknown_logic() {
  let dir = test_dir();
  let path = dir.path().join("quoted.fst");
  fixture(&path);
  let result = records(invoke(
    &path,
    r#"SELECT CASE WHEN "top.bus [7:0]" > 5 THEN raw("top.quote""λ\name") ELSE 'early' END FROM samples"#,
    &[],
  ));
  assert_eq!(
    rows(&result),
    vec![json!(["early"]), json!(["00XZ"]), json!(["00XZ"])]
  );
}

#[test]
fn grouping_and_ordering_resolve_input_paths_separately_from_output_aliases() {
  let dir = test_dir();
  let path = dir.path().join("quoted.fst");
  fixture(&path);
  let result = records(invoke(
    &path,
    r#"SELECT "top.bus [7:0]" % 2 AS "parity", COUNT(*) AS n FROM samples GROUP BY "top.bus [7:0]" % 2 ORDER BY "parity""#,
    &[],
  ));
  assert_eq!(rows(&result), vec![json!(["0", "1"]), json!(["1", "2"])]);
  let result = records(invoke(
    &path,
    r#"SELECT tick FROM samples ORDER BY samples."top.bus [7:0]" DESC LIMIT 1"#,
    &[],
  ));
  assert_eq!(rows(&result), vec![json!(["8"])]);
  let result = records(invoke(
    &path,
    r#"SELECT tick AS "top.unused" FROM samples ORDER BY "top.unused" DESC LIMIT 1"#,
    &[],
  ));
  assert_eq!(rows(&result), vec![json!(["8"])]);
  assert_eq!(result.last().unwrap()["decoded_callbacks"], "0");
}

#[test]
fn explicit_bindings_take_precedence_and_unquoted_paths_are_not_inferred() {
  let dir = test_dir();
  let path = dir.path().join("quoted.fst");
  fixture(&path);
  let result = records(invoke(
    &path,
    r#"SELECT "top.bus [7:0]", "tick" FROM samples LIMIT 1"#,
    &["--bind", "top.bus [7:0]=top.other"],
  ));
  assert_eq!(rows(&result), vec![json!(["9", "0"])]);
  for sql in [
    r#"SELECT "missing.signal" FROM samples"#,
    "SELECT top.other FROM samples",
    "SELECT missing FROM samples",
  ] {
    let output = invoke(&path, sql, &[]);
    assert!(!output.status.success(), "{sql}");
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "query_error");
  }
}

#[test]
fn ambiguous_flattened_paths_are_rejected_instead_of_merging_values() {
  let dir = test_dir();
  let path = dir.path().join("ambiguous.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  writer.set_scope(scope_type::VCD_MODULE, "top", "").unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "a.b", None)
    .unwrap();
  writer.set_scope(scope_type::VCD_MODULE, "a", "").unwrap();
  let b = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "b", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(a, b"0").unwrap();
  writer.emit_value_change(b, b"1").unwrap();
  writer.emit_time_change(4).unwrap();
  drop(writer);
  let output = invoke(&path, r#"SELECT "top.a.b" FROM samples"#, &[]);
  assert!(!output.status.success());
  let error: Value = serde_json::from_slice(&output.stderr).unwrap();
  assert!(
    error["message"]
      .as_str()
      .unwrap()
      .contains("ambiguous signal path")
  );
}
