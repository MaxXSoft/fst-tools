use fstapi::{Writer, scope_type, var_dir, var_type};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture(PathBuf);

impl Drop for Fixture {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

#[test]
fn finds_exact_hex_regex_and_alias_filtered_values() {
  let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("findfst-{}", std::process::id()));
  fs::create_dir_all(&dir).unwrap();
  let fixture = Fixture(dir);
  let path = fixture.0.join("input.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  writer
    .set_scope(scope_type::VCD_MODULE, "top", "top")
    .unwrap();
  let handle = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, "data", None)
    .unwrap();
  writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, "alias", Some(handle))
    .unwrap();
  writer.set_upscope();
  for (time, value) in [(0, b"0011"), (10, b"1010"), (20, b"0011")] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(handle, value).unwrap();
  }
  writer.emit_time_change(30).unwrap();
  drop(writer);

  let run = |args: &[&str]| {
    let result = Command::new(env!("CARGO_BIN_EXE_findfst"))
      .arg(&path)
      .args(args)
      .output()
      .unwrap();
    assert!(
      result.status.success(),
      "{}",
      String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
  };
  assert_eq!(run(&["11"]), "#0 top.data 0011\n");
  assert_eq!(
    run(&["11", "--all-matches"]),
    "#0 top.data 0011\n#20 top.data 0011\n"
  );
  assert_eq!(run(&["a", "--hex"]), "#10 top.data 1010\n");
  assert_eq!(
    run(&["^00", "--regex", "--all-matches"]),
    "#0 top.data 0011\n#20 top.data 0011\n"
  );
  assert_eq!(
    run(&["3", "--hex", "--regex", "--names-only"]),
    "top.data\n"
  );
  assert_eq!(
    run(&["11", "--signals", "^top\\.alias$", "--names-only"]),
    "top.alias\n"
  );
  assert_eq!(run(&["1111"]), "");

  let invalid = Command::new(env!("CARGO_BIN_EXE_findfst"))
    .arg(&path)
    .arg("not-binary")
    .output()
    .unwrap();
  assert!(!invalid.status.success());
}
