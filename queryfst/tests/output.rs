use fstapi::{Writer, var_dir, var_type};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

fn fixture(path: &Path) {
  let mut writer = Writer::create(path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "a", None)
    .unwrap();
  for t in 0..100 {
    writer.emit_time_change(t).unwrap();
    writer
      .emit_value_change(a, if t % 2 == 0 { b"0" } else { b"1" })
      .unwrap();
  }
  writer.emit_time_change(100).unwrap();
  writer.emit_value_change(a, b"0").unwrap();
}
fn invoke(path: &Path, args: &[&str]) -> Output {
  Command::new(env!("CARGO_BIN_EXE_queryfst"))
    .arg(path)
    .args(args)
    .output()
    .unwrap()
}
fn records(output: &Output) -> Vec<Value> {
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  String::from_utf8_lossy(&output.stdout)
    .lines()
    .map(|s| serde_json::from_str(s).unwrap())
    .collect()
}
#[test]
fn default_text_and_json_shortcut() {
  let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
  let path = dir.path().join("input.fst");
  fixture(&path);
  let text = invoke(&path, &["--signal", "a", "--limit", "1"]);
  assert!(text.status.success());
  assert!(String::from_utf8_lossy(&text.stdout).starts_with("FST query:"));
  let json = invoke(&path, &["--signal", "a", "--json", "--limit", "1"]);
  assert_eq!(records(&json).last().unwrap()["truncated"], true);
}
#[test]
fn callback_budget_stops_backend_and_distinguishes_partial_from_truncated_output() {
  let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
  let path = dir.path().join("input.fst");
  fixture(&path);
  for cap in [0, 1, 5] {
    let output = invoke(
      &path,
      &[
        "--signal",
        "a",
        "--json",
        "--max-callbacks",
        &cap.to_string(),
      ],
    );
    let r = records(&output);
    let summary = r.last().unwrap();
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["decoded_callbacks"], cap.to_string());
    assert_eq!(summary["unprocessed_input"], true);
  }
  let output = invoke(
    &path,
    &["--signal", "a", "--json", "--max-duration-ms", "0"],
  );
  let r = records(&output);
  assert_eq!(r.last().unwrap()["decoded_callbacks"], "0");
  assert_eq!(r.last().unwrap()["complete"], false);
}
#[test]
fn whole_stdout_byte_budget_leaves_a_complete_footer() {
  let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
  let path = dir.path().join("input.fst");
  fixture(&path);
  for format in ["json", "text"] {
    let output = invoke(
      &path,
      &["--signal", "a", "--format", format, "--max-bytes", "4096"],
    );
    assert!(output.status.success());
    assert!(output.stdout.len() <= 4096);
    if format == "json" {
      let r = records(&output);
      let summary = r.last().unwrap();
      assert_eq!(summary["type"], "summary");
      assert_eq!(summary["complete"], true);
      assert_eq!(summary["output_truncated"], true);
    }
  }
}
#[test]
fn structured_argument_and_execution_errors() {
  let output = invoke(Path::new("missing.fst"), &["--json"]);
  assert!(!output.status.success());
  let error: Value = serde_json::from_slice(&output.stderr).unwrap();
  assert_eq!(error["code"], "invalid_arguments");
  let output = invoke(Path::new("missing.fst"), &["--json", "--signal", "a"]);
  assert!(!output.status.success());
  let error: Value = serde_json::from_slice(&output.stderr).unwrap();
  assert_eq!(error["code"], "input_error");
}

#[test]
fn first_and_last_expand_match_context_without_losing_the_trigger() {
  let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
  let path = dir.path().join("input.fst");
  fixture(&path);
  for (mode, expected, complete_scan) in [
    ("first", vec![0, 1, 2], false),
    ("last", vec![98, 99, 100], true),
  ] {
    let output = invoke(
      &path,
      &[
        "--json",
        "--bind",
        "a=a",
        "--period",
        "1",
        "--sql",
        "SELECT tick, a FROM samples WHERE a = 1",
        "--matches",
        mode,
        "--before",
        "1",
        "--after",
        "1",
      ],
    );
    let result = records(&output);
    let rows: Vec<_> = result.iter().filter(|r| r["type"] == "row").collect();
    assert_eq!(
      rows
        .iter()
        .map(|r| r["values"][0].as_str().unwrap().parse::<u64>().unwrap())
        .collect::<Vec<_>>(),
      expected
    );
    assert_eq!(
      rows
        .iter()
        .map(|r| r["values"][2].as_bool().unwrap())
        .collect::<Vec<_>>(),
      [false, true, false]
    );
    assert_eq!(result.last().unwrap()["complete"], true);
    assert_eq!(result.last().unwrap()["scan_complete"], complete_scan);
  }
  let invalid = invoke(
    &path,
    &[
      "--json",
      "--bind",
      "a=a",
      "--period",
      "1",
      "--sql",
      "SELECT tick FROM samples WHERE a = 1 LIMIT 1",
      "--before",
      "1",
    ],
  );
  assert!(!invalid.status.success());
  assert_eq!(
    serde_json::from_slice::<Value>(&invalid.stderr).unwrap()["type"],
    "error"
  );
}

#[test]
fn sql_only_options_require_sql_even_when_explicitly_equal_to_defaults() {
  for (flag, value) in [
    ("--bindings", "map.json"),
    ("--bind", "a=a"),
    ("--period", "1"),
    ("--phase", "0"),
    ("--max-samples", "0"),
    ("--max-groups", "100000"),
    ("--max-buffer-rows", "100000"),
    ("--matches", "all"),
    ("--before", "0"),
    ("--after", "0"),
  ] {
    let output = invoke(
      Path::new("missing.fst"),
      &["--signal", "a", "--json", flag, value],
    );
    assert_eq!(output.status.code(), Some(2), "{flag}");
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "invalid_arguments", "{flag}");
    assert!(output.stdout.is_empty());
  }
  let missing_period = invoke(
    Path::new("missing.fst"),
    &["--sql", "SELECT tick FROM samples", "--json"],
  );
  assert_eq!(missing_period.status.code(), Some(2));
}

#[test]
fn diagnostics_respect_terminator_and_help() {
  let output = Command::new(env!("CARGO_BIN_EXE_queryfst"))
    .args(["--", "--json"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(2));
  assert!(String::from_utf8_lossy(&output.stderr).starts_with("error:"));
  for format in ["--format=json", "--format=jsonl", "--json"] {
    let output = invoke(Path::new("missing.fst"), &[format]);
    assert_eq!(
      serde_json::from_slice::<Value>(&output.stderr).unwrap()["code"],
      "invalid_arguments"
    );
    let help = invoke(Path::new("missing.fst"), &[format, "--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
  }
}

#[test]
fn binding_diagnostics_identify_the_rejected_argument() {
  let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
  let path = dir.path().join("input.fst");
  fixture(&path);
  for binding in ["broken binding", "=a", "a="] {
    let output = invoke(
      &path,
      &[
        "--sql",
        "SELECT tick FROM samples",
        "--period",
        "1",
        "--bind",
        binding,
        "--json",
      ],
    );
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["message"].as_str().unwrap().contains(binding));
  }
}
