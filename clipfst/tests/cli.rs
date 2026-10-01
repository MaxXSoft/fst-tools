use fstapi::{Reader, Writer, scope_type, var_dir, var_type};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
  dir: PathBuf,
  input: PathBuf,
}

impl Fixture {
  fn new() -> Self {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
      "clipfst-{}-{}",
      std::process::id(),
      NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.fst");
    let mut writer = Writer::create(&input, true).unwrap().timezero(-7);
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
    Self { dir, input }
  }

  fn clip(&self, name: &str, args: &[&str]) -> PathBuf {
    let output = self.dir.join(name);
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

impl Drop for Fixture {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.dir);
  }
}

const VECTORS: [&[u8]; 4] = [b"10001000", b"00111001", b"11111111", b"00000000"];
const REALS: [f64; 4] = [1.25, -2.5, 0.125, 17.0];
const PORTS: [&[u8]; 4] = [b"01 66 77", b"10 77 66", b"11 66 66", b"00 77 77"];
type Value = (u64, Vec<u8>, bool);

fn read_values(path: &Path) -> (Reader, BTreeMap<String, Vec<Value>>) {
  let mut reader = Reader::open(path).unwrap();
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
      .arg(fixture.dir.join("invalid.fst"))
      .args(&args)
      .output()
      .unwrap();
    assert!(!result.status.success());
  }
}
