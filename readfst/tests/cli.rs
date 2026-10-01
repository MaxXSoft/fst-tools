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
fn displays_metadata_hierarchy_attributes_and_aliases() {
  let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("readfst-{}", std::process::id()));
  fs::create_dir_all(&dir).unwrap();
  let fixture = Fixture(dir);
  let path = fixture.0.join("input.fst");
  let mut writer = Writer::create(&path, true)
    .unwrap()
    .version("readfst regression")
    .unwrap()
    .comment("fixture comment")
    .unwrap();
  writer
    .set_scope(scope_type::VCD_MODULE, "top", "module_type")
    .unwrap();
  let handle = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, "data", None)
    .unwrap();
  writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, "alias", Some(handle))
    .unwrap();
  writer.set_upscope();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(handle, b"0011").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);

  let run = |args: &[&str]| {
    let result = Command::new(env!("CARGO_BIN_EXE_readfst"))
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
  let all = run(&["--all"]);
  for expected in [
    "Metadata",
    "Variables",
    "Scopes",
    "Attributes",
    "readfst regression",
    "fixture comment",
    "top.data",
    "top.alias",
    "module_type",
  ] {
    assert!(all.contains(expected), "missing {expected:?} in {all}");
  }
  assert_eq!(run(&["--vars", "--names-only"]), "top.data\ntop.alias\n");
  assert_eq!(
    run(&["--vars", "--names-only", "--no-aliases"]),
    "top.data\n"
  );

  let missing = Command::new(env!("CARGO_BIN_EXE_readfst"))
    .arg(fixture.0.join("missing.fst"))
    .arg("--all")
    .output()
    .unwrap();
  assert!(!missing.status.success());
}

#[test]
fn displays_system_verilog_array_scope() {
  let dir =
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("readfst-array-{}", std::process::id()));
  fs::create_dir_all(&dir).unwrap();
  let fixture = Fixture(dir);
  let path = fixture.0.join("array.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  writer
    .set_scope(scope_type::VCD_MODULE, "top", "top")
    .unwrap();
  writer
    .set_scope(scope_type::SV_ARRAY, "memory", "")
    .unwrap();
  let handle = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "[0]", None)
    .unwrap();
  writer.set_upscope();
  writer.set_upscope();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(handle, b"10101010").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);

  let result = Command::new(env!("CARGO_BIN_EXE_readfst"))
    .arg(&path)
    .arg("--all")
    .output()
    .unwrap();
  assert!(
    result.status.success(),
    "{}",
    String::from_utf8_lossy(&result.stderr)
  );
  let output = String::from_utf8(result.stdout).unwrap();
  assert!(output.contains("SvArray"), "{output}");
  assert!(output.contains("top.memory.[0]"), "{output}");
}
