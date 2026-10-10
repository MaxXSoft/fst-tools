use fstapi::{Writer, attr_type, misc_type, scope_type, var_dir, var_type};
use std::ffi::CString;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn test_dir() -> TempDir {
  let test_binary = std::env::current_exe().unwrap();
  tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap()
}

fn json_output(path: &Path, args: &[&str]) -> serde_json::Value {
  let result = Command::new(env!("CARGO_BIN_EXE_readfst"))
    .arg(path)
    .args(["--format", "json"])
    .args(args)
    .output()
    .unwrap();
  assert!(
    result.status.success(),
    "{}",
    String::from_utf8_lossy(&result.stderr)
  );
  assert!(result.stderr.is_empty());
  serde_json::from_slice(&result.stdout).unwrap()
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
      .set_attr_begin_raw(attr_type::MISC, subtype as _, &name, line)
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
  let json = json_output(&path, &["--attrs"]);
  let stems: Vec<_> = json["attributes"]
    .as_array()
    .unwrap()
    .iter()
    .filter_map(|attr| attr.get("data"))
    .filter(|data| matches!(data["subtype"].as_str(), Some("SourceStem" | "SourceIStem")))
    .collect();
  assert_eq!(stems.len(), 2);
  assert_eq!(stems[0]["name"], "");
  assert_eq!(stems[0]["arg"], "300");
  assert_eq!(stems[0]["arg_from_name"], "128");
  assert_eq!(stems[1]["arg_from_name"], "129");
}

#[test]
fn json_preserves_large_times_escaping_paths_aliases_and_attribute_boundaries() {
  let dir = test_dir();
  let path = dir.path().join("json.fst");
  let mut writer = Writer::create(&path, true)
    .unwrap()
    .version("version \"quoted\" \\ path")
    .unwrap()
    .timescale(-12)
    .timezero(-9_007_199_254_740_993);
  writer
    .set_scope(scope_type::VCD_MODULE, "top.with.dot", "component\"\\")
    .unwrap();
  writer
    .set_scope(scope_type::VCD_MODULE, "inner", "")
    .unwrap();
  writer
    .set_attr_begin(
      attr_type::MISC,
      misc_type::COMMENT,
      "comment \"\\\n雪",
      u64::MAX,
    )
    .unwrap();
  let signal = "data\"\\雪";
  let handle = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 4, signal, None)
    .unwrap();
  writer
    .create_var(var_type::VCD_REG, var_dir::INPUT, 4, "alias", Some(handle))
    .unwrap();
  writer.set_attr_end();
  writer.set_upscope();
  writer.set_upscope();
  writer.emit_time_change(9_007_199_254_740_995).unwrap();
  writer.emit_value_change(handle, b"0011").unwrap();
  writer.emit_time_change(9_007_199_254_741_001).unwrap();
  drop(writer);

  let json = json_output(&path, &["--all"]);
  assert_eq!(json["schema"], "readfst");
  assert_eq!(json["schema_version"], 1);
  assert_eq!(json["metadata"]["version"], "version \"quoted\" \\ path");
  assert_eq!(json["metadata"]["file_type_code"], 0);
  assert_eq!(json["metadata"]["timescale_exponent"], -12);
  assert_eq!(json["metadata"]["timezero"], "-9007199254740993");
  assert_eq!(json["metadata"]["start_time"], "9007199254740995");
  assert_eq!(json["metadata"]["end_time"], "9007199254741001");
  assert_eq!(json["metadata"]["num_vars"], "2");
  let vars = json["variables"].as_array().unwrap();
  assert_eq!(vars.len(), 2);
  assert_eq!(
    vars[0]["path"],
    serde_json::json!(["top.with.dot", "inner", signal])
  );
  assert_eq!(vars[0]["width"], 4);
  assert_eq!(vars[0]["type_code"], 5);
  assert_eq!(vars[0]["direction_code"], 2);
  assert_eq!(vars[0]["handle"], u32::from(handle));
  assert_eq!(vars[0]["is_alias"], false);
  assert!(vars[0]["alias_of"].is_null());
  assert_eq!(vars[1]["handle"], vars[0]["handle"]);
  assert_eq!(vars[1]["is_alias"], true);
  assert_eq!(vars[1]["type_code"], 5);
  assert_eq!(vars[1]["direction_code"], 1);
  assert_eq!(vars[1]["alias_of"], vars[0]["name"]);
  assert_eq!(vars[1]["canonical_name"], vars[0]["name"]);
  assert_eq!(
    json["scopes"][1]["path"],
    serde_json::json!(["top.with.dot", "inner"])
  );
  assert_eq!(json["scopes"][1]["full_name"], "top.with.dot.inner");
  assert_eq!(json["scopes"][1]["type_code"], 0);
  let attrs = json["attributes"].as_array().unwrap();
  assert!(attrs.iter().any(|attr| attr["event"] == "begin"
    && attr["data"]["name"] == "comment \"\\\n雪"
    && attr["data"]["arg"] == "18446744073709551615"));
  assert!(
    attrs
      .iter()
      .any(|attr| attr["event"] == "end" && attr.get("data").is_none())
  );
  assert_eq!(json, json_output(&path, &["--all"]));
}

#[test]
fn filtered_alias_retains_canonical_name_in_json_and_table_modes() {
  let dir = test_dir();
  let path = dir.path().join("filter.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  writer.set_scope(scope_type::VCD_MODULE, "top", "").unwrap();
  let handle = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "data", None)
    .unwrap();
  writer
    .create_var(var_type::VCD_WIRE, var_dir::INPUT, 1, "alias", Some(handle))
    .unwrap();
  writer.set_upscope();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(handle, b"0").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);

  let json = json_output(&path, &["--vars", "--signals", "^top\\.alias$"]);
  assert_eq!(json["variables"].as_array().unwrap().len(), 1);
  assert_eq!(json["variables"][0]["canonical_name"], "top.data");
  assert_eq!(json["variables"][0]["alias_of"], "top.data");
  for absent in ["metadata", "scopes", "attributes"] {
    assert!(json.get(absent).is_none());
  }
  let no_alias = json_output(&path, &["--vars", "--signals", "alias$", "--no-aliases"]);
  assert_eq!(no_alias["variables"], serde_json::json!([]));
  for (extra, expected) in [
    (&["--names-only"][..], "top.alias\n"),
    (&[][..], "top.data"),
  ] {
    let result = Command::new(env!("CARGO_BIN_EXE_readfst"))
      .arg(&path)
      .args(["--vars", "-S", "alias$"])
      .args(extra)
      .output()
      .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8(result.stdout).unwrap().contains(expected));
  }
}

#[test]
fn rejects_incompatible_json_options_and_invalid_filters_before_opening_input() {
  for args in [
    vec!["--format", "json"],
    vec!["--vars", "--format", "json", "--names-only"],
    vec!["--metadata", "--signals", "data"],
    vec!["--vars", "--signals", "["],
  ] {
    let result = Command::new(env!("CARGO_BIN_EXE_readfst"))
      .arg("does-not-exist.fst")
      .args(args)
      .output()
      .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("context creation"));
  }
}

#[test]
fn json_selected_empty_sections_are_arrays() {
  let dir = test_dir();
  let path = dir.path().join("empty-json.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let json = json_output(&path, &["--all"]);
  assert_eq!(json["variables"], serde_json::json!([]));
  assert_eq!(json["scopes"], serde_json::json!([]));
  assert_eq!(json["attributes"], serde_json::json!([]));
  let metadata = json_output(&path, &["--metadata"]);
  assert!(metadata.get("variables").is_none());
  assert!(metadata.get("scopes").is_none());
  assert!(metadata.get("attributes").is_none());
}
