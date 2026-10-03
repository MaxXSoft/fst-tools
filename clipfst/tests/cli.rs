use fstapi::{Hier, MiscType, Reader, Writer, attr_type, misc_type, scope_type, var_dir, var_type};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct Fixture {
  dir: TempDir,
  input: PathBuf,
}

impl Fixture {
  fn empty() -> Self {
    let test_binary = std::env::current_exe().unwrap();
    let dir = tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap();
    let input = dir.path().join("input.fst");
    Self { dir, input }
  }

  fn new() -> Self {
    let fixture = Self::empty();
    let input = &fixture.input;
    let mut writer = Writer::create(input, true).unwrap().timezero(-7);
    writer
      .set_scope(scope_type::VCD_MODULE, "top", "top")
      .unwrap();
    let vector = writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "vector", None)
      .unwrap();
    let real = writer
      .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 64, "real", None)
      .unwrap();
    let string = writer
      .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "string", None)
      .unwrap();
    let port = writer
      .create_var(var_type::VCD_PORT, var_dir::INOUT, 8, "port", None)
      .unwrap();
    writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "alias", Some(vector))
      .unwrap();
    writer.set_upscope();
    for (index, time) in [0, 10, 20, 30].into_iter().enumerate() {
      writer.emit_time_change(time).unwrap();
      writer.emit_value_change(vector, VECTORS[index]).unwrap();
      writer
        .emit_value_change(real, &REALS[index].to_ne_bytes())
        .unwrap();
      writer.emit_value_change(port, PORTS[index]).unwrap();
      if index == 0 || index == 3 {
        writer
          .emit_var_len_value_change(string, if index == 0 { b"a\0b" } else { b"z\0end" })
          .unwrap();
      }
      // Make the unchanged string at t=0 cross a value-change block boundary.
      if index == 1 {
        writer.flush();
      }
    }
    writer.emit_time_change(40).unwrap();
    drop(writer);
    fixture
  }

  fn clip(&self, name: &str, args: &[&str]) -> PathBuf {
    let output = self.dir.path().join(name);
    let result = Command::new(env!("CARGO_BIN_EXE_clipfst"))
      .arg(&self.input)
      .arg(&output)
      .args(args)
      .output()
      .unwrap();
    assert!(
      result.status.success(),
      "clipfst {args:?}: {}",
      String::from_utf8_lossy(&result.stderr)
    );
    output
  }
}

const VECTORS: [&[u8]; 4] = [b"10001000", b"00111001", b"11111111", b"00000000"];
const REALS: [f64; 4] = [1.25, -2.5, 0.125, 17.0];
const PORTS: [&[u8]; 4] = [b"01 66 77", b"10 77 66", b"11 66 66", b"00 77 77"];
type Value = (u64, Vec<u8>, bool);

fn read_values(path: &Path) -> (Reader, BTreeMap<String, Vec<Value>>) {
  let mut reader = Reader::open(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
  let names: BTreeMap<_, _> = reader
    .vars()
    .filter_map(|entry| {
      let (name, var) = entry.unwrap();
      (!var.is_alias()).then_some((var.handle(), name))
    })
    .collect();
  let mut values: BTreeMap<String, Vec<Value>> = BTreeMap::new();
  reader.set_mask_all();
  reader.set_native_doubles_on_callback(true);
  reader
    .for_each_block(|time, handle, value, var_len| {
      values
        .entry(names[&handle].clone())
        .or_default()
        .push((time, value.to_vec(), var_len));
    })
    .unwrap();
  (reader, values)
}

#[test]
fn copies_real_values_as_native_doubles() {
  let fixture = Fixture::new();
  let path = fixture.clip("real.fst", &["-S", "^real$"]);
  let (_, values) = read_values(&path);
  assert_eq!(
    values["top.real"],
    REALS
      .into_iter()
      .enumerate()
      .map(|(index, value)| { (index as u64 * 10, value.to_ne_bytes().to_vec(), false) })
      .collect::<Vec<_>>()
  );
}

#[test]
fn copies_evcd_port_storage_width() {
  let fixture = Fixture::new();
  let path = fixture.clip("port.fst", &["-S", "^port$"]);
  let (mut reader, values) = read_values(&path);
  assert_eq!(reader.vars().next().unwrap().unwrap().1.length(), 2);
  assert_eq!(
    values["top.port"],
    PORTS
      .into_iter()
      .enumerate()
      .map(|(index, value)| { (index as u64 * 10, value.to_vec(), false) })
      .collect::<Vec<_>>()
  );
}

#[test]
fn clipped_window_preserves_initial_values_aliases_and_timezero() {
  let fixture = Fixture::new();
  for pack in ["4", "f", "z"] {
    let path = fixture.clip(
      &format!("clip-{pack}.fst"),
      &[
        "--start",
        "5",
        "--end",
        "15",
        "--pack-type",
        pack,
        "--no-comp-hier",
        "--repack",
        "--parallel",
      ],
    );
    let (reader, values) = read_values(&path);
    assert_eq!(
      (reader.start_time(), reader.end_time(), reader.timezero()),
      (0, 10, -2)
    );
    assert_eq!((reader.var_count(), reader.alias_count()), (5, 1));
    assert_eq!(
      values["top.vector"],
      vec![
        (0, VECTORS[0].to_vec(), false),
        (5, VECTORS[1].to_vec(), false)
      ]
    );
    assert_eq!(
      values["top.real"],
      vec![
        (0, REALS[0].to_ne_bytes().to_vec(), false),
        (5, REALS[1].to_ne_bytes().to_vec(), false)
      ]
    );
    assert_eq!(
      values["top.port"],
      vec![(0, PORTS[0].to_vec(), false), (5, PORTS[1].to_vec(), false)]
    );
    assert_eq!(values["top.string"], vec![(0, b"a\0b".to_vec(), true)]);
  }
}

#[test]
fn quiet_window_keeps_values_from_earlier_blocks() {
  let fixture = Fixture::new();
  let path = fixture.clip("quiet.fst", &["--start", "21", "--end", "25"]);
  let (reader, values) = read_values(&path);
  assert_eq!(
    (reader.start_time(), reader.end_time(), reader.timezero()),
    (0, 4, 14)
  );
  assert_eq!(values["top.vector"], vec![(0, VECTORS[2].to_vec(), false)]);
  assert_eq!(
    values["top.real"],
    vec![(0, REALS[2].to_ne_bytes().to_vec(), false)]
  );
  assert_eq!(values["top.port"], vec![(0, PORTS[2].to_vec(), false)]);
  assert_eq!(values["top.string"], vec![(0, b"a\0b".to_vec(), true)]);
}

#[test]
fn string_window_keeps_variable_length_initial_values() {
  let fixture = Fixture::new();
  let path = fixture.clip(
    "string.fst",
    &["-S", "^string$", "--start", "5", "--end", "35"],
  );
  let (reader, values) = read_values(&path);
  assert_eq!((reader.start_time(), reader.end_time()), (0, 30));
  assert_eq!(
    values["top.string"],
    vec![(0, b"a\0b".to_vec(), true), (25, b"z\0end".to_vec(), true)]
  );
}

#[test]
fn unchanged_selected_vector_survives_a_quiet_window() {
  let fixture = Fixture::new();
  let path = fixture.clip(
    "quiet-vector.fst",
    &["-S", "^vector$", "--start", "21", "--end", "25"],
  );
  let (reader, values) = read_values(&path);
  assert_eq!((reader.start_time(), reader.end_time()), (0, 4));
  assert_eq!(values["top.vector"], vec![(0, VECTORS[2].to_vec(), false)]);
}

#[test]
fn zero_duration_window_uses_values_at_the_boundary() {
  let fixture = Fixture::new();
  let path = fixture.clip("point.fst", &["--start", "30", "--end", "30"]);
  let (reader, values) = read_values(&path);
  assert_eq!(
    (reader.start_time(), reader.end_time(), reader.timezero()),
    (0, 0, 23)
  );
  assert_eq!(values["top.vector"], vec![(0, VECTORS[3].to_vec(), false)]);
  assert_eq!(values["top.string"], vec![(0, b"z\0end".to_vec(), true)]);
}

#[test]
fn alias_only_selection_keeps_its_value() {
  let fixture = Fixture::new();
  let path = fixture.clip(
    "alias.fst",
    &["-S", "^alias$", "--start", "5", "--end", "15"],
  );
  let (reader, values) = read_values(&path);
  assert_eq!((reader.var_count(), reader.alias_count()), (1, 0));
  assert_eq!(
    values["top.alias"],
    vec![
      (0, VECTORS[0].to_vec(), false),
      (5, VECTORS[1].to_vec(), false)
    ]
  );
}

#[test]
fn invalid_windows_and_missing_signals_report_failure() {
  let fixture = Fixture::new();
  for args in [vec!["--start", "15", "--end", "5"], vec!["-S", "^missing$"]] {
    let result = Command::new(env!("CARGO_BIN_EXE_clipfst"))
      .arg(&fixture.input)
      .arg(fixture.dir.path().join("invalid.fst"))
      .args(&args)
      .output()
      .unwrap();
    assert!(!result.status.success());
  }
}

#[test]
fn failed_in_place_selection_preserves_input_without_extra_files() {
  let fixture = Fixture::new();
  let files = || {
    fs::read_dir(fixture.dir.path())
      .unwrap()
      .map(|entry| entry.unwrap().path())
      .collect::<Vec<_>>()
  };
  assert_eq!(files(), vec![fixture.input.clone()], "fixture cleanup");
  let original = fs::read(&fixture.input).unwrap();
  let result = Command::new(env!("CARGO_BIN_EXE_clipfst"))
    .arg(&fixture.input)
    .arg(&fixture.input)
    .args(["-S", "^missing$"])
    .output()
    .unwrap();
  assert!(!result.status.success());
  assert_eq!(fs::read(&fixture.input).unwrap(), original);
  assert_eq!(files(), vec![fixture.input.clone()], "clipfst cleanup");
}

#[test]
fn rejects_input_aliases_before_creating_output() {
  let fixture = Fixture::new();
  let original = fs::read(&fixture.input).unwrap();
  let hardlink = fixture.dir.path().join("hardlink.fst");
  fs::hard_link(&fixture.input, &hardlink).unwrap();
  let outputs = vec![fixture.input.clone(), hardlink];
  #[cfg(unix)]
  let outputs = {
    let mut outputs = outputs;
    let symlink = fixture.dir.path().join("symlink.fst");
    std::os::unix::fs::symlink(&fixture.input, &symlink).unwrap();
    outputs.push(symlink);
    outputs
  };
  for output in outputs {
    let result = Command::new(env!("CARGO_BIN_EXE_clipfst"))
      .arg(&fixture.input)
      .arg(&output)
      .args(["--start", "5", "--end", "15"])
      .output()
      .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("different files"));
    assert_eq!(fs::read(&fixture.input).unwrap(), original);
    assert_eq!(fs::read(&output).unwrap(), original);
  }
}

#[test]
fn preserves_all_changes_at_the_start_boundary() {
  for start in [0, 10] {
    let fixture = Fixture::empty();
    let mut writer = Writer::create(&fixture.input, true).unwrap();
    let value = writer
      .create_var(var_type::VCD_EVENT, var_dir::OUTPUT, 1, "event", None)
      .unwrap();
    writer.emit_time_change(0).unwrap();
    writer.emit_value_change(value, b"0").unwrap();
    writer.emit_time_change(start).unwrap();
    for value_change in [b"1", b"0", b"1"] {
      writer.emit_value_change(value, value_change).unwrap();
    }
    writer.emit_time_change(20).unwrap();
    drop(writer);
    let (_, input_values) = read_values(&fixture.input);
    let expected: Vec<_> = input_values["event"]
      .iter()
      .filter(|(time, _, _)| *time == start)
      .map(|(_, value, variable)| (0, value.clone(), *variable))
      .collect();
    assert!(
      expected.len() >= 3,
      "fixture must contain same-time changes"
    );
    let path = fixture.clip("boundary.fst", &["--start", &start.to_string()]);
    let (_, values) = read_values(&path);
    assert_eq!(values["event"], expected);
  }
}

#[test]
fn preserves_binary_source_attributes() {
  let fixture = Fixture::empty();
  let mut writer = Writer::create(&fixture.input, true).unwrap();
  writer
    .set_attr_begin(attr_type::MISC, misc_type::PATHNAME as _, "source.sv", 128)
    .unwrap();
  for subtype in [misc_type::SOURCESTEM, misc_type::SOURCEISTEM] {
    writer
      .set_attr_begin_raw(
        attr_type::MISC,
        subtype as _,
        std::ffi::CStr::from_bytes_with_nul(&[0x80, 1, 0]).unwrap(),
        42,
      )
      .unwrap();
  }
  let signal = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "signal", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(signal, b"1").unwrap();
  writer.emit_time_change(1).unwrap();
  drop(writer);
  let output = fixture.clip("attributes.fst", &[]);
  let mut reader = Reader::open(output).unwrap();
  let sources: Vec<_> = reader
    .hiers()
    .filter_map(|hier| match hier {
      Hier::AttrBegin(attr)
        if matches!(
          attr.subtype() as MiscType,
          misc_type::SOURCESTEM | misc_type::SOURCEISTEM
        ) =>
      {
        Some((
          attr.subtype() as MiscType,
          attr.name_cstr().to_bytes().to_vec(),
          attr.arg_from_name(),
          attr.arg(),
        ))
      }
      _ => None,
    })
    .collect();
  assert_eq!(
    sources,
    vec![
      (misc_type::SOURCESTEM, vec![0x80, 1], 128, 42),
      (misc_type::SOURCEISTEM, vec![0x80, 1], 128, 42),
    ]
  );
}

#[test]
fn preserves_and_clips_dump_activity_even_without_value_changes() {
  let fixture = Fixture::empty();
  let mut writer = Writer::create(&fixture.input, true).unwrap();
  let signal = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "signal", None)
    .unwrap();
  writer.emit_time_change(0).unwrap();
  writer.emit_value_change(signal, b"1").unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_dump_active(false).unwrap();
  writer.emit_time_change(20).unwrap();
  writer.emit_dump_active(true).unwrap();
  writer.emit_time_change(30).unwrap();
  drop(writer);
  for (start, end, expected) in [
    (0, 30, vec![(10, false), (20, true)]),
    (5, 25, vec![(5, false), (15, true)]),
    (15, 25, vec![(0, false), (5, true)]),
    (15, 18, vec![(0, false)]),
    (10, 10, vec![(0, false)]),
    (20, 20, vec![(0, true)]),
    (25, 30, vec![]),
  ] {
    let path = fixture.clip(
      &format!("activity-{start}-{end}.fst"),
      &["--start", &start.to_string(), "--end", &end.to_string()],
    );
    let (reader, values) = read_values(&path);
    assert_eq!(reader.dump_activity(), expected, "window {start}-{end}");
    assert_eq!(reader.end_time(), end - start);
    assert_eq!(values["signal"], vec![(0, b"1".to_vec(), false)]);
  }
}
