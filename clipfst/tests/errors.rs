use fstapi::{Writer, var_dir, var_type};
use std::fs;
use std::process::Command;

#[test]
fn overflowing_timezero_reports_context_before_creating_output() {
  let test_binary = std::env::current_exe().unwrap();
  let dir = tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap();
  let input = dir.path().join("input.fst");
  let output = dir.path().join("output.fst");
  let mut writer = Writer::create(&input, true).unwrap().timezero(i64::MAX);
  let bit = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
    .unwrap();
  writer.emit_value_change(bit, b"1").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  // A rejected clip must leave a pre-existing output intact.
  fs::write(&output, b"existing output").unwrap();
  let result = Command::new(env!("CARGO_BIN_EXE_clipfst"))
    .arg(&input)
    .arg(&output)
    .args(["--start", "1"])
    .output()
    .unwrap();
  assert!(!result.status.success());
  let stderr = String::from_utf8(result.stderr).unwrap();
  assert!(
    stderr.contains("Clipped timezero exceeds the signed 64-bit range"),
    "{stderr}"
  );
  assert!(stderr.contains(&format!("{} + 1", i64::MAX)), "{stderr}");
  assert_eq!(fs::read(output).unwrap(), b"existing output");
}
