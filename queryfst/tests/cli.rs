use fstapi::{Reader, Writer, scope_type, var_dir, var_type};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Keeps generated waveforms inside Cargo's ignored build directory.
fn test_dir() -> TempDir {
  tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap()
}

/// Creates aliases, four-state/wide vectors, arbitrary bytes and scalar changes.
fn fixture(path: &Path) {
  let mut writer = Writer::create(path, true).unwrap();
  writer.set_scope(scope_type::VCD_MODULE, "top", "").unwrap();
  let flag = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "flag", None)
    .unwrap();
  writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "alias", Some(flag))
    .unwrap();
  let wide = writer
    .create_var(var_type::SV_LOGIC, var_dir::OUTPUT, 132, "wide", None)
    .unwrap();
  let bytes = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "bytes", None)
    .unwrap();
  writer.set_upscope();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(flag, b"0").unwrap();
  writer
    .emit_value_change(wide, format!("{}xz01", "1".repeat(128)).as_bytes())
    .unwrap();
  writer
    .emit_var_len_value_change(bytes, &[0, 255, b'\n', b'"'])
    .unwrap();
  for (time, value) in [(5, b"1"), (10, b"0"), (15, b"x"), (20, b"z"), (30, b"1")] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(flag, value).unwrap();
  }
  writer.emit_time_change(40).unwrap();
}

/// Executes the CLI without involving a shell or collecting an unbounded trace.
fn invoke(path: &Path, args: &[&str]) -> Output {
  Command::new(env!("CARGO_BIN_EXE_queryfst"))
    .arg(path)
    .args(args)
    .output()
    .unwrap()
}

/// Parses every output line and requires successful completion.
fn run(path: &Path, args: &[&str]) -> Vec<Value> {
  let mut json_args = vec!["--format", "json"];
  json_args.extend_from_slice(args);
  let output = invoke(path, &json_args);
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

#[test]
fn bounded_query_preserves_prior_state_aliases_and_inclusive_endpoints() {
  let dir = test_dir();
  let path = dir.path().join("input.fst");
  fixture(&path);
  let records = run(
    &path,
    &[
      "--signal",
      "top.alias",
      "--start",
      "10",
      "--end",
      "20",
      "--limit",
      "2",
    ],
  );
  let signal = records.iter().find(|r| r["type"] == "signal").unwrap();
  assert_eq!(signal["path"], "top.flag");
  assert_eq!(signal["aliases"], serde_json::json!(["top.alias"]));
  let initial = records.iter().find(|r| r["type"] == "initial").unwrap();
  assert_eq!(initial["time"], "10");
  assert_eq!(initial["source_time"], "5");
  assert_eq!(initial["value"], "1");
  let events: Vec<_> = records.iter().filter(|r| r["type"] == "event").collect();
  assert_eq!(events.len(), 2);
  assert_eq!(
    (&events[0]["time"], &events[0]["value"]),
    (&Value::from("10"), &Value::from("0"))
  );
  assert_eq!(
    (&events[1]["time"], &events[1]["value"]),
    (&Value::from("15"), &Value::from("x"))
  );
  let summary = records.last().unwrap();
  assert_eq!(summary["matching_callbacks"], "3");
  assert_eq!(summary["emitted_events"], "2");
  assert_eq!(summary["omitted_events"], "1");
  assert_eq!(summary["truncated"], true);
  // The callback reader decodes the rest of the enclosing block, so queryfst
  // must filter the post-end callback at 30 itself.
  assert!(
    summary["decoded_callbacks"]
      .as_str()
      .unwrap()
      .parse::<u64>()
      .unwrap()
      > 3
  );
  let full = run(
    &path,
    &["--signal", "top.flag", "--start", "10", "--end", "20"],
  );
  let last_event = full.iter().rev().find(|r| r["type"] == "event").unwrap();
  assert_eq!(last_event["time"], "20");
  assert_eq!(last_event["value"], "z");
  assert_eq!(full.last().unwrap()["truncated"], false);
}

#[test]
fn lossless_values_and_zero_event_limit() {
  let dir = test_dir();
  let path = dir.path().join("input.fst");
  fixture(&path);
  let records = run(&path, &["--signals", "top\\.(wide|bytes)$"]);
  let values: Vec<_> = records.iter().filter(|r| r["type"] == "event").collect();
  assert_eq!(values.len(), 2);
  let bits = values.iter().find(|r| r["encoding"] == "bits").unwrap();
  assert_eq!(bits["value"], format!("{}xz01", "1".repeat(128)));
  let bytes = values
    .iter()
    .find(|r| r["encoding"] == "bytes_hex")
    .unwrap();
  assert_eq!(bytes["value"], "00ff0a22");
  let empty = run(&path, &["--signal", "top.flag", "--limit", "0"]);
  assert!(empty.iter().all(|r| r["type"] != "event"));
  assert_eq!(empty.last().unwrap()["truncated"], true);
}

#[test]
fn real_values_preserve_ieee_bits_without_decimal_rounding() {
  let dir = test_dir();
  let path = dir.path().join("reals.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let real = writer
    .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 1, "real", None)
    .unwrap();
  // Adjacent to 1.0: formatting with 16 significant digits loses the low bit.
  let bits = 0x3ff0_0000_0000_0001u64;
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(real, &bits.to_ne_bytes()).unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let records = run(&path, &["--signal", "real"]);
  let event = records.iter().find(|r| r["type"] == "event").unwrap();
  assert_eq!(event["encoding"], "real_f64_le_hex");
  assert_eq!(event["value"], "010000000000f03f");
}

#[test]
fn scalar_summary_accounts_elapsed_ticks_and_endpoint_transitions() {
  let dir = test_dir();
  let path = dir.path().join("input.fst");
  fixture(&path);
  let records = run(
    &path,
    &[
      "--signals",
      "flag|alias",
      "--start",
      "7",
      "--end",
      "20",
      "--summary",
    ],
  );
  let summary = records
    .iter()
    .find(|r| r["type"] == "scalar_summary")
    .unwrap();
  assert_eq!(summary["duration_ticks"], "13");
  assert_eq!(
    summary["residency_ticks"],
    serde_json::json!({
      "0": "5", "1": "3", "x": "5", "z": "0", "other": "0", "unavailable": "0"
    })
  );
  assert_eq!(summary["value_transitions"], "3");
  assert_eq!(summary["callbacks"], "3");
  assert_eq!(records.last().unwrap()["selected_handles"], 1);
  assert_eq!(records.last().unwrap()["truncated"], false);
  assert!(records.iter().all(|r| r["type"] != "event"));
}

#[test]
fn rejects_missing_paths_empty_matches_invalid_ranges_and_non_scalars() {
  let dir = test_dir();
  let path = dir.path().join("input.fst");
  fixture(&path);
  for args in [
    vec![],
    vec!["--signals", "missing"],
    vec!["--signal", "top.flag", "--signal", "missing"],
    vec!["--signals", "["],
    vec!["--signal", "top.flag", "--start", "21", "--end", "20"],
    vec!["--signal", "top.flag", "--end", "41"],
    vec!["--signal", "top.wide", "--summary"],
  ] {
    let output = invoke(&path, &args);
    assert!(!output.status.success(), "{args:?}");
    assert!(output.stdout.is_empty(), "{args:?}");
    assert!(!output.stderr.is_empty(), "{args:?}");
  }
}

#[test]
fn preserves_backend_equal_timestamp_order_and_large_timestamp_strings() {
  let dir = test_dir();
  let path = dir.path().join("large-time.fst");
  let time = 9_007_199_254_740_993;
  let mut writer = Writer::create(&path, true).unwrap();
  let a = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "a", None)
    .unwrap();
  let b = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "b", None)
    .unwrap();
  writer.emit_time_change(time).unwrap();
  writer.emit_value_change(a, b"0").unwrap();
  writer.emit_value_change(b, b"1").unwrap();
  writer.emit_time_change(time + 1).unwrap();
  writer.emit_value_change(a, b"1").unwrap();
  writer.emit_value_change(b, b"0").unwrap();
  writer.emit_time_change(time + 2).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  reader.set_mask_all();
  let mut expected = Vec::new();
  reader
    .for_each_block(|time, handle, value, _| {
      expected.push((
        time.to_string(),
        u32::from(handle),
        String::from_utf8(value.to_vec()).unwrap(),
      ));
    })
    .unwrap();
  let records = run(&path, &["--signals", "."]);
  let actual: Vec<_> = records
    .iter()
    .filter(|r| r["type"] == "event")
    .enumerate()
    .map(|(i, r)| {
      assert_eq!(r["sequence"], i.to_string());
      (
        r["time"].as_str().unwrap().to_owned(),
        r["handle"].as_u64().unwrap() as u32,
        r["value"].as_str().unwrap().to_owned(),
      )
    })
    .collect();
  assert_eq!(actual, expected);
  assert_eq!(records[0]["start"], time.to_string());
}

#[test]
fn variable_values_are_recovered_across_skipped_blocks() {
  let dir = test_dir();
  let path = dir.path().join("blocks.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let flag = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "flag", None)
    .unwrap();
  let bytes = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "bytes", None)
    .unwrap();
  for time in 0..100 {
    writer.emit_time_change(time).unwrap();
    writer
      .emit_value_change(flag, if time % 2 == 0 { b"0" } else { b"1" })
      .unwrap();
    if time == 0 {
      writer.emit_var_len_value_change(bytes, b"prior").unwrap();
    }
    if time == 50 {
      writer.flush();
    }
  }
  writer.emit_time_change(100).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  reader.set_mask(bytes);
  reader.set_time_range_limit(80, 90);
  let mut byte_callbacks = 0;
  reader
    .for_each_block(|_, _, _, _| byte_callbacks += 1)
    .unwrap();
  assert_eq!(byte_callbacks, 0, "fixture must skip the first block");
  let records = run(
    &path,
    &["--signal", "bytes", "--start", "80", "--end", "90"],
  );
  assert_eq!(records[0]["scan_start"], "0");
  let initial = records.iter().find(|r| r["type"] == "initial").unwrap();
  assert_eq!(initial["value"], "7072696f72");
  assert_eq!(initial["source_time"], "0");
  assert_eq!(records.last().unwrap()["matching_callbacks"], "0");
  let fixed = run(&path, &["--signal", "flag", "--start", "80", "--end", "90"]);
  assert_eq!(fixed[0]["scan_start"], "80");
  let initial = fixed.iter().find(|r| r["type"] == "initial").unwrap();
  assert_eq!(initial["source_time"], "79");
  assert_eq!(initial["value"], "1");
  assert_eq!(fixed.last().unwrap()["matching_callbacks"], "11");
  // A flush at 50 opens the next block at 51. The backend may emit a frame
  // snapshot at its boundary; applying every event at 51 reconstructs state.
  let boundary = run(&path, &["--signal", "flag", "--start", "51", "--end", "51"]);
  let mut observed = boundary.iter().find(|r| r["type"] == "initial").unwrap()["value"].clone();
  for event in boundary.iter().filter(|r| r["type"] == "event") {
    assert_eq!(event["time"], "51");
    observed = event["value"].clone();
  }
  assert_eq!(observed, "1");
  let point = run(
    &path,
    &[
      "--signal",
      "flag",
      "--start",
      "51",
      "--end",
      "51",
      "--summary",
    ],
  );
  let summary = point
    .iter()
    .find(|r| r["type"] == "scalar_summary")
    .unwrap();
  assert_eq!(summary["duration_ticks"], "0");
  assert!(
    summary["residency_ticks"]
      .as_object()
      .unwrap()
      .values()
      .all(|v| v == "0")
  );
}

#[test]
fn recording_gaps_are_explicit_and_not_counted_as_state_residency() {
  let dir = test_dir();
  let path = dir.path().join("gap.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let flag = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "flag", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(flag, b"1").unwrap();
  writer.emit_time_change(5).unwrap();
  writer.emit_dump_active(false).unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_dump_active(true).unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);
  let output = invoke(&path, &["--signal", "flag", "--summary"]);
  assert!(!output.status.success());
  assert!(output.stdout.is_empty());
  let records = run(&path, &["--signal", "flag", "--start", "12"]);
  assert_eq!(
    records
      .iter()
      .filter(|r| r["type"] == "dump_activity")
      .count(),
    2
  );
  let initial = records.iter().find(|r| r["type"] == "initial").unwrap();
  assert_eq!(initial["value"], Value::Null);
  let summaries = run(&path, &["--signal", "flag", "--start", "12", "--summary"]);
  let summary = summaries
    .iter()
    .find(|r| r["type"] == "scalar_summary")
    .unwrap();
  assert_eq!(summary["residency_ticks"]["unavailable"], "8");
}

#[test]
fn block_snapshots_do_not_hide_a_gap_before_the_requested_range() {
  let dir = test_dir();
  let path = dir.path().join("gap-blocks.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let flag = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "flag", None)
    .unwrap();
  let tick = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "tick", None)
    .unwrap();
  for time in 0..100 {
    writer.emit_time_change(time).unwrap();
    writer
      .emit_value_change(tick, if time % 2 == 0 { b"0" } else { b"1" })
      .unwrap();
    match time {
      0 => writer.emit_value_change(flag, b"1").unwrap(),
      5 => writer.emit_dump_active(false).unwrap(),
      10 => writer.emit_dump_active(true).unwrap(),
      50 => writer.flush(),
      _ => (),
    }
  }
  writer.emit_time_change(100).unwrap();
  drop(writer);
  let records = run(
    &path,
    &[
      "--signal",
      "flag",
      "--start",
      "80",
      "--end",
      "90",
      "--summary",
    ],
  );
  assert_eq!(records[0]["scan_start"], "0");
  let initial = records.iter().find(|r| r["type"] == "initial").unwrap();
  assert_eq!(initial["value"], Value::Null);
  let summary = records
    .iter()
    .find(|r| r["type"] == "scalar_summary")
    .unwrap();
  assert_eq!(summary["residency_ticks"]["unavailable"], "10");
}

#[test]
fn zero_duration_dump_interruptions_also_invalidate_state() {
  let dir = test_dir();
  let path = dir.path().join("zero-gap.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let flag = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "flag", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(flag, b"1").unwrap();
  writer.emit_time_change(5).unwrap();
  writer.emit_dump_active(false).unwrap();
  writer.emit_dump_active(true).unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);
  for range in [vec![], vec!["--start", "5", "--end", "5"]] {
    let mut args = vec!["--signal", "flag", "--summary"];
    args.extend(range);
    let output = invoke(&path, &args);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
  }
  let records = run(&path, &["--signal", "flag", "--start", "12", "--summary"]);
  let initial = records.iter().find(|r| r["type"] == "initial").unwrap();
  assert_eq!(initial["value"], Value::Null);
  let summary = records
    .iter()
    .find(|r| r["type"] == "scalar_summary")
    .unwrap();
  assert_eq!(summary["residency_ticks"]["unavailable"], "8");
}
