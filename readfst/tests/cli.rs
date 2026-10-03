use fstapi::{Writer, attr_type, misc_type, scope_type, var_dir, var_type};
use std::ffi::CString;
use std::process::Command;
use tempfile::TempDir;

fn test_dir() -> TempDir {
  let test_binary = std::env::current_exe().unwrap();
  tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap()
}

#[test]
fn displays_metadata_hierarchy_attributes_and_aliases() {
  let dir = test_dir();
  let path = dir.path().join("input.fst");
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
    .arg(dir.path().join("missing.fst"))
    .arg("--all")
    .output()
    .unwrap();
  assert!(!missing.status.success());
}

#[test]
fn displays_system_verilog_array_scope() {
  let dir = test_dir();
  let path = dir.path().join("array.fst");
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

#[test]
fn displays_empty_variable_table_without_aliases() {
  let dir = test_dir();
  let path = dir.path().join("empty.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);

  for args in [
    vec!["--vars", "--no-aliases"],
    vec!["--all", "--no-aliases"],
  ] {
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
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("Variables"), "{output}");
    assert!(output.contains("None"), "{output}");
  }
}

#[test]
fn displays_binary_source_stem_attribute_arguments() {
  let dir = test_dir();
  let path = dir.path().join("sources.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  for (subtype, source, line) in [
    (misc_type::SOURCESTEM, 128, 300),
    (misc_type::SOURCEISTEM, 129, 400),
  ] {
    let name = CString::new(vec![source, 1]).unwrap();
    writer
      .set_attr_begin_raw(attr_type::MISC, subtype, &name, line)
      .unwrap();
  }
  writer.set_scope(scope_type::VCD_MODULE, "top", "").unwrap();
  let handle = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "data", None)
    .unwrap();
  writer.set_upscope();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(handle, b"0").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);

  let result = Command::new(env!("CARGO_BIN_EXE_readfst"))
    .arg(&path)
    .arg("--attrs")
    .output()
    .unwrap();
  assert!(
    result.status.success(),
    "{}",
    String::from_utf8_lossy(&result.stderr)
  );
  let output = String::from_utf8(result.stdout).unwrap();
  for expected in [
    ["Misc", "SourceStem", "300", "128"],
    ["Misc", "SourceIStem", "400", "129"],
  ] {
    assert!(
      output
        .lines()
        .any(|line| line.trim_matches('│').split_whitespace().eq(expected)),
      "{output}"
    );
  }
}
