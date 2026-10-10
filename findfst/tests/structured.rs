use fstapi::{Writer, scope_type, var_dir, var_type};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn test_dir() -> TempDir {
  tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap()
}

fn fixture(path: &Path) {
  let mut writer = Writer::create(path, true)
    .unwrap()
    .timescale(-12)
    .timezero(-9_007_199_254_740_993);
  writer.set_scope(scope_type::VCD_MODULE, "top", "").unwrap();
  let a = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, "a", None)
    .unwrap();
  writer
    .create_var(var_type::VCD_REG, var_dir::INPUT, 4, "alias", Some(a))
    .unwrap();
  let b = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, "b", None)
    .unwrap();
  writer.set_upscope();
  for (time, value) in [(0, b"0011"), (10, b"1010"), (20, b"0011"), (30, b"1010")] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(a, value).unwrap();
    writer.emit_value_change(b, value).unwrap();
  }
  writer.emit_time_change(40).unwrap();
}

fn invoke(path: &Path, args: &[&str]) -> Output {
  Command::new(env!("CARGO_BIN_EXE_findfst"))
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
  assert!(
    output.stderr.is_empty(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  let records: Vec<Value> = String::from_utf8(output.stdout.clone())
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect();
  assert_eq!(records.first().unwrap()["type"], "header");
  assert_eq!(records.last().unwrap()["type"], "summary");
  assert_eq!(
    records.last().unwrap()["stdout_bytes"],
    output.stdout.len().to_string()
  );
  records
}

#[test]
fn json_preserves_first_per_handle_and_alias_selection() {
  let dir = test_dir();
  let path = dir.path().join("first.fst");
  fixture(&path);
  let output = invoke(&path, &["11", "--format", "json"]);
  let records = records(&output);
  assert_eq!(records[0]["schema"], "findfst");
  assert_eq!(records[0]["schema_version"], 1);
  assert_eq!(records[0]["mode"], "first_per_handle");
  assert_eq!(records[0]["timescale_exponent"], -12);
  assert_eq!(records[0]["timezero"], "-9007199254740993");
  let matches: Vec<_> = records.iter().filter(|r| r["type"] == "match").collect();
  assert_eq!(matches.len(), 2);
  assert!(
    matches
      .iter()
      .all(|r| r["time"] == "0" && r["value"] == "0011")
  );
  assert_eq!(records.last().unwrap()["total_matches"], "2");
  let records = self::records(&invoke(
    &path,
    &["11", "--json", "--signals", "^top\\.alias$"],
  ));
  let signal = records.iter().find(|r| r["type"] == "signal").unwrap();
  assert_eq!(signal["name"], "top.alias");
  assert_eq!(signal["canonical_name"], "top.a");
  assert_eq!(signal["aliases"], json!(["top.alias"]));
  assert_eq!(
    records.iter().find(|r| r["type"] == "match").unwrap()["name"],
    "top.alias"
  );
  assert_eq!(records.last().unwrap()["selected_handles"], 1);
}

#[test]
fn window_is_inclusive_and_prior_matches_do_not_consume_first_result() {
  let dir = test_dir();
  let path = dir.path().join("range.fst");
  fixture(&path);
  let args = ["11", "--json", "--start", "10", "--end", "20"];
  let records = records(&invoke(&path, &args));
  let matches: Vec<_> = records.iter().filter(|r| r["type"] == "match").collect();
  assert_eq!(matches.len(), 2);
  assert!(matches.iter().all(|r| r["time"] == "20"));
  let point = self::records(&invoke(
    &path,
    &["11", "--json", "--start", "20", "--end", "20"],
  ));
  assert_eq!(point.last().unwrap()["total_matches"], "2");
  let quiet = self::records(&invoke(
    &path,
    &["11", "--json", "--start", "1", "--end", "9"],
  ));
  assert_eq!(quiet.last().unwrap()["total_matches"], "0");
  // This is callback matching, not a carried-in level query.
  assert_eq!(quiet[0]["match_semantics"], "in_range_callbacks_only");
  let text = invoke(&path, &["11", "--start", "10", "--end", "20"]);
  assert!(text.status.success());
  assert!(
    String::from_utf8(text.stdout)
      .unwrap()
      .lines()
      .all(|line| line.starts_with("#20 "))
  );
}

#[test]
fn row_budget_retains_complete_totals_and_does_not_duplicate_aliases() {
  let dir = test_dir();
  let path = dir.path().join("rows.fst");
  fixture(&path);
  let output = invoke(
    &path,
    &[
      "11",
      "--format",
      "jsonl",
      "--all-matches",
      "--max-rows",
      "1",
    ],
  );
  let records = records(&output);
  let footer = records.last().unwrap();
  assert_eq!(records.iter().filter(|r| r["type"] == "match").count(), 1);
  assert_eq!(footer["execution_complete"], true);
  assert_eq!(footer["status"], "complete");
  assert_eq!(footer["output_truncated"], true);
  assert_eq!(footer["total_matches"], "4");
  assert_eq!(footer["total_omitted_matches"], "3");
  let zero = self::records(&invoke(&path, &["11", "--json", "--max-rows", "0"]));
  assert_eq!(zero.last().unwrap()["total_matches"], "2");
  assert_eq!(zero.last().unwrap()["emitted_matches"], "0");
}

#[test]
fn callback_and_duration_budgets_are_partial_with_unknown_totals() {
  let dir = test_dir();
  let path = dir.path().join("work.fst");
  fixture(&path);
  for (flag, limit, reason, callbacks) in [
    ("--max-callbacks", "0", "callback_budget_exhausted", "0"),
    ("--max-callbacks", "1", "callback_budget_exhausted", "1"),
    ("--max-duration-ms", "0", "duration_budget_exhausted", "0"),
  ] {
    let records = records(&invoke(&path, &["1111", "--json", flag, limit]));
    let footer = records.last().unwrap();
    assert_eq!(footer["status"], "partial");
    assert_eq!(footer["execution_complete"], false);
    assert_eq!(footer["stop_reason"], reason);
    assert_eq!(footer["decoded_callbacks"], callbacks);
    assert!(footer["total_matches"].is_null());
    assert!(footer["total_omitted_matches"].is_null());
    assert_eq!(footer["observed_matches"], "0");
  }
  let both = records(&invoke(
    &path,
    &["11", "--json", "--max-callbacks", "1", "--max-rows", "0"],
  ));
  let footer = both.last().unwrap();
  assert_eq!(footer["execution_complete"], false);
  assert_eq!(footer["output_truncated"], true);
  assert_eq!(footer["observed_omitted_matches"], "1");
}

#[test]
fn whole_stdout_byte_budget_accounts_for_metadata_footer_and_large_values() {
  let dir = test_dir();
  let path = dir.path().join("bytes.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let h = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "text", None)
    .unwrap();
  for i in 0..80 {
    writer
      .create_var(
        var_type::GEN_STRING,
        var_dir::OUTPUT,
        0,
        &format!("alias_{i}_{}", "n".repeat(100)),
        Some(h),
      )
      .unwrap();
  }
  writer.emit_time_change(0).unwrap();
  writer.emit_var_len_value_change(h, b"short").unwrap();
  writer.emit_time_change(5).unwrap();
  writer
    .emit_var_len_value_change(h, &vec![b'x'; 8000])
    .unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_var_len_value_change(h, b"last").unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);
  let output = invoke(
    &path,
    &[
      ".*",
      "--regex",
      "--all-matches",
      "--json",
      "--max-bytes",
      "4096",
    ],
  );
  assert!(output.stdout.len() <= 4096);
  let records = records(&output);
  let footer = records.last().unwrap();
  assert_eq!(footer["execution_complete"], true);
  assert_eq!(footer["metadata_complete"], false);
  assert_eq!(footer["omitted_metadata_records"], "1");
  assert_eq!(footer["emitted_matches"], "1");
  assert_eq!(footer["total_matches"], "3");
  assert_eq!(footer["total_omitted_matches"], "2");
  let text = invoke(
    &path,
    &[".*", "--regex", "--all-matches", "--max-bytes", "4096"],
  );
  assert!(text.status.success());
  assert_eq!(text.stdout, b"#0 text short\n");
  assert!(String::from_utf8_lossy(&text.stderr).contains("output_truncated=true"));
}

#[test]
fn json_values_preserve_arbitrary_bytes_names_and_large_times() {
  let dir = test_dir();
  let path = dir.path().join("escaping.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let name = "quote\"\\雪";
  let h = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, name, None)
    .unwrap();
  let time = 9_007_199_254_740_993u64;
  writer.emit_time_change(time).unwrap();
  writer
    .emit_var_len_value_change(h, &[0xff, 0, b'\n', b'"'])
    .unwrap();
  writer.emit_time_change(time + 1).unwrap();
  drop(writer);
  let records = records(&invoke(&path, &[".*", "--regex", "--json"]));
  let record = records.iter().find(|r| r["type"] == "match").unwrap();
  assert_eq!(record["time"], time.to_string());
  assert_eq!(record["name"], name);
  assert_eq!(record["encoding"], "bytes_hex");
  assert_eq!(record["value"], "ff000a22");
}

#[test]
fn json_errors_cover_parse_validation_input_and_output_failures() {
  let dir = test_dir();
  let path = dir.path().join("errors.fst");
  fixture(&path);
  for args in [
    vec!["bad-value", "--json"],
    vec!["1", "--json", "--unrecognized"],
    vec!["1", "--format=json", "--names-only"],
    vec!["1", "--json", "--max-bytes", "12"],
    vec!["1", "--json", "--start", "30", "--end", "20"],
    vec!["1", "--json", "--end", "41"],
  ] {
    let output = invoke(&path, &args);
    assert_eq!(output.status.code(), Some(2), "{args:?}");
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["type"], "error");
    assert_eq!(error["code"], "invalid_arguments");
  }
  let missing = invoke(&dir.path().join("missing.fst"), &["1", "--json"]);
  assert_eq!(missing.status.code(), Some(1));
  let error: Value = serde_json::from_slice(&missing.stderr).unwrap();
  assert_eq!(error["code"], "input_error");
  #[cfg(unix)]
  {
    use std::net::Shutdown;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;
    let (output, receiver) = UnixStream::pair().unwrap();
    // Make writes fail even if another process still holds a peer descriptor.
    output.shutdown(Shutdown::Write).unwrap();
    drop(receiver);
    let output = Command::new(env!("CARGO_BIN_EXE_findfst"))
      .arg(&path)
      .args(["11", "--json"])
      .stdout(Stdio::from(OwnedFd::from(output)))
      .output()
      .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "output_error");
  }
}

#[test]
fn empty_selection_is_a_completed_empty_search() {
  let dir = test_dir();
  let path = dir.path().join("empty.fst");
  fixture(&path);
  let records = records(&invoke(&path, &["11", "--json", "--signals", "^missing$"]));
  let footer = records.last().unwrap();
  assert_eq!(footer["execution_complete"], true);
  assert_eq!(footer["selected_handles"], 0);
  assert_eq!(footer["total_matches"], "0");
  assert_eq!(footer["decoded_callbacks"], "0");
}

#[test]
fn wide_four_state_values_and_native_reals_keep_their_encodings() {
  let dir = test_dir();
  let path = dir.path().join("value-kinds.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let wide = writer
    .create_var(var_type::SV_LOGIC, var_dir::OUTPUT, 132, "wide", None)
    .unwrap();
  let real = writer
    .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 1, "real", None)
    .unwrap();
  let bits = format!("{}xz01", "1".repeat(128));
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(wide, bits.as_bytes()).unwrap();
  writer
    .emit_value_change(real, &0x3ff0_0000_0000_0001u64.to_ne_bytes())
    .unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let records = records(&invoke(&path, &[".*", "--regex", "--json"]));
  let wide = records
    .iter()
    .find(|r| r["type"] == "match" && r["name"] == "wide")
    .unwrap();
  assert_eq!(wide["encoding"], "bits");
  assert_eq!(wide["value"], bits);
  assert_eq!(wide["width"], 132);
  let real = records
    .iter()
    .find(|r| r["type"] == "match" && r["name"] == "real")
    .unwrap();
  assert_eq!(real["encoding"], "real_f64_le_hex");
  assert_eq!(real["value"], "010000000000f03f");
  let text = invoke(&path, &[".*", "--regex", "--signals", "^real$"]);
  assert!(text.status.success());
  assert_eq!(
    String::from_utf8(text.stdout).unwrap(),
    "#0 real 1.0000000000000002\n"
  );
}

#[test]
fn crossing_window_end_is_complete_before_a_later_work_budget_expiry() {
  let dir = test_dir();
  let path = dir.path().join("past-end.fst");
  fixture(&path);
  let records = records(&invoke(
    &path,
    &["11", "--json", "--end", "5", "--max-callbacks", "3"],
  ));
  let footer = records.last().unwrap();
  assert_eq!(footer["complete"], true);
  assert_eq!(footer["execution_complete"], true);
  assert!(footer["reason"].is_null());
  assert_eq!(footer["unprocessed_input"], false);
  assert_eq!(footer["total_matches"], "2");
  assert_eq!(footer["last_callback_time"], "10");
  assert_eq!(footer["processed_through"], "5");
  let partial = self::records(&invoke(&path, &["11", "--json", "--max-callbacks", "3"]));
  let footer = partial.last().unwrap();
  assert_eq!(footer["complete"], false);
  assert_eq!(footer["last_callback_time"], "10");
  assert_eq!(footer["processed_through"], "9");
}

#[test]
fn argument_diagnostics_respect_terminator_and_help() {
  let output = Command::new(env!("CARGO_BIN_EXE_findfst"))
    .args(["--", "--json"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(2));
  assert!(String::from_utf8_lossy(&output.stderr).starts_with("error:"));
  let output = invoke(Path::new("missing.fst"), &["1", "--json", "--help"]);
  assert!(output.status.success());
  assert!(output.stderr.is_empty());
}

#[test]
fn real_matches_preserve_round_trip_values_signed_zero_and_nan_payloads() {
  let dir = test_dir();
  let path = dir.path().join("real-bits.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let real = writer
    .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 1, "real", None)
    .unwrap();
  let values = [
    0x3ff0_0000_0000_0001u64,
    0x8000_0000_0000_0000,
    0x7ff0_0000_0000_0000,
    0x7ff8_0000_0000_0042,
    0xfff8_0000_0000_0081,
  ];
  for (tick, bits) in values.iter().enumerate() {
    writer.emit_time_change(tick as u64).unwrap();
    writer.emit_value_change(real, &bits.to_ne_bytes()).unwrap();
  }
  writer.emit_time_change(values.len() as u64).unwrap();
  drop(writer);
  let output = invoke(&path, &[".*", "--regex", "--all-matches", "--json"]);
  let result = records(&output);
  let matches: Vec<_> = result.iter().filter(|r| r["type"] == "match").collect();
  assert_eq!(matches.len(), values.len());
  for (record, bits) in matches.iter().zip(values) {
    let expected = bits
      .to_le_bytes()
      .iter()
      .map(|b| format!("{b:02x}"))
      .collect::<String>();
    assert_eq!(record["encoding"], "real_f64_le_hex");
    assert_eq!(record["value"], expected);
  }
  for text in [
    "1.0000000000000002",
    "-0",
    "inf",
    "NaN(0x7ff8000000000042)",
    "NaN(0xfff8000000000081)",
  ] {
    let pattern = format!("^{}$", regex::escape(text));
    let output = invoke(&path, &[&pattern, "--regex", "--json"]);
    assert_eq!(
      records(&output).last().unwrap()["total_matches"],
      "1",
      "{text}"
    );
  }
}
