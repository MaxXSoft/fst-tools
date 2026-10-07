mod common;

use common::*;
use fstapi::{
  Error, LimitKind, Reader, ValueKind, VariableDefinitionError, Writer, var_dir, var_type,
};

fn rejected_value_preserves_initial_time(invalid_first: bool) {
  let dir = TestDir::new();
  let path = dir.path("rejected-value.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let handle = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
    .unwrap();
  if !invalid_first {
    writer.emit_value_change(handle, b"1").unwrap();
  }
  assert_eq!(
    writer.emit_value_change(handle, b"10"),
    Err(Error::ValueLengthMismatch(handle, 1, 2))
  );
  if invalid_first {
    writer.emit_value_change(handle, b"1").unwrap();
  }
  writer.emit_time_change(10).unwrap();
  writer.emit_value_change(handle, b"0").unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);

  let mut reader = Reader::open(&path).unwrap();
  assert_eq!(
    reader.start_time(),
    0,
    "rejected input must not discard the initial value at time zero"
  );
  reader.set_mask_all();
  assert_eq!(
    events(&mut reader),
    vec![
      Event {
        time: 0,
        handle,
        value: b"1".to_vec(),
        variable_length: false
      },
      Event {
        time: 10,
        handle,
        value: b"0".to_vec(),
        variable_length: false
      },
    ]
  );
}

#[test]
fn rejected_value_does_not_change_existing_initial_value_time() {
  rejected_value_preserves_initial_time(false);
}

#[test]
fn rejected_value_does_not_change_subsequent_initial_value_time() {
  rejected_value_preserves_initial_time(true);
}

#[test]
fn wrong_emitter_and_unknown_handles_are_rejected_without_side_effects() {
  let dir = TestDir::new();
  let path = dir.path("invalid-operations.fst");
  let mut other = Writer::create(dir.path("other.fst"), true).unwrap();
  other
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "first", None)
    .unwrap();
  let unknown = other
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "second", None)
    .unwrap();
  let mut writer = Writer::create(&path, true).unwrap();
  let bit = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
    .unwrap();
  writer.emit_value_change(bit, b"1").unwrap();
  assert_eq!(
    writer.emit_value_change(unknown, b"1"),
    Err(Error::InvalidHandle(unknown))
  );
  assert_eq!(
    writer.create_var(
      var_type::VCD_REG,
      var_dir::OUTPUT,
      1,
      "bad-alias",
      Some(unknown)
    ),
    Err(Error::InvalidHandle(unknown))
  );
  assert_eq!(
    writer.emit_var_len_value_change(bit, b"a"),
    Err(Error::ValueKindMismatch(
      bit,
      ValueKind::Fixed,
      ValueKind::Variable
    ))
  );
  writer.emit_time_change(10).unwrap();
  assert_eq!(
    writer.emit_time_change(9),
    Err(Error::TimeWentBackwards(10, 9))
  );
  writer.emit_value_change(bit, b"0").unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  assert_eq!(reader.var_count(), 1);
  assert_eq!(reader.start_time(), 0);
  reader.set_mask_all();
  assert_eq!(
    events(&mut reader)
      .iter()
      .map(|e| (e.time, e.value.clone()))
      .collect::<Vec<_>>(),
    vec![(0, b"1".to_vec()), (10, b"0".to_vec())]
  );
}

#[test]
fn aliases_use_normalized_storage_and_reject_incompatible_widths() {
  let dir = TestDir::new();
  let path = dir.path("aliases.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let real = writer
    .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 64, "real", None)
    .unwrap();
  let alias = writer
    .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 1, "alias", Some(real))
    .unwrap();
  assert_eq!(alias, real);
  assert_eq!(
    writer.create_var(
      var_type::VCD_REG,
      var_dir::OUTPUT,
      8,
      "bad-alias",
      Some(real)
    ),
    Err(Error::IncompatibleAlias(real))
  );
  let string = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 32, "string", None)
    .unwrap();
  assert_eq!(
    writer.emit_value_change(string, b"wrong emitter"),
    Err(Error::ValueKindMismatch(
      string,
      ValueKind::Variable,
      ValueKind::Fixed
    ))
  );
  assert_eq!(
    writer.emit_value_change(real, b"1"),
    Err(Error::ValueLengthMismatch(real, 8, 1))
  );
  writer.emit_time_change(0).unwrap();
  writer
    .emit_value_change(alias, &1.25_f64.to_ne_bytes())
    .unwrap();
  writer.emit_var_len_value_change(string, b"a\0b").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  assert_eq!(reader.var_count(), 3);
  assert_eq!(reader.alias_count(), 1);
  reader.set_mask_all();
  let actual = events(&mut reader);
  assert_eq!(actual.len(), 2);
  assert_eq!(actual[0].value, b"1.25");
  assert_eq!(actual[1].value, b"a\0b");
}

#[test]
fn variable_length_initial_values_survive_the_first_time_change() {
  let dir = TestDir::new();
  let path = dir.path("initial-string.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let string = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "string", None)
    .unwrap();
  writer.emit_var_len_value_change(string, b"a\0b").unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_var_len_value_change(string, b"").unwrap();
  writer.emit_time_change(20).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  assert_eq!(reader.start_time(), 0);
  reader.set_mask_all();
  assert_eq!(
    events(&mut reader),
    vec![
      Event {
        time: 0,
        handle: string,
        value: b"a\0b".to_vec(),
        variable_length: true
      },
      Event {
        time: 10,
        handle: string,
        value: Vec::new(),
        variable_length: true
      },
    ]
  );
}

#[test]
fn static_initial_values_are_readable_after_time_advances() {
  let dir = TestDir::new();
  for end in [0, 10] {
    let path = dir.path(&format!("static-{end}.fst"));
    let mut writer = Writer::create(&path, true).unwrap();
    let bit = writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
      .unwrap();
    let real = writer
      .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 64, "real", None)
      .unwrap();
    writer.emit_value_change(bit, b"1").unwrap();
    writer
      .emit_value_change(real, &0.125_f64.to_ne_bytes())
      .unwrap();
    if end != 0 {
      writer.emit_time_change(end).unwrap();
    }
    drop(writer);
    let mut reader = Reader::open(&path).unwrap();
    assert_eq!((reader.start_time(), reader.end_time()), (0, end));
    reader.set_mask_all();
    assert_eq!(
      events(&mut reader),
      vec![
        Event {
          time: 0,
          handle: bit,
          value: b"1".to_vec(),
          variable_length: false
        },
        Event {
          time: 0,
          handle: real,
          value: b"0.125".to_vec(),
          variable_length: false
        },
      ]
    );
  }
}

#[test]
fn evcd_ports_require_a_complete_value_and_two_strength_vectors() {
  let dir = TestDir::new();
  let path = dir.path("port-width.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  for width in [0, 1, 2, 3, 4, 6, 7, 9] {
    assert_eq!(
      writer.create_var(var_type::VCD_PORT, var_dir::INOUT, width, "invalid", None),
      Err(Error::InvalidVariableDefinition(
        VariableDefinitionError::InvalidPortWidth(width)
      ))
    );
  }
  let one = writer
    .create_var(var_type::VCD_PORT, var_dir::INOUT, 5, "one", None)
    .unwrap();
  let two = writer
    .create_var(var_type::VCD_PORT, var_dir::INOUT, 8, "two", None)
    .unwrap();
  writer.emit_value_change(one, b"0 6 7").unwrap();
  writer.emit_value_change(two, b"01 66 77").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  assert_eq!(reader.var_count(), 2);
  reader.set_mask_all();
  assert_eq!(
    events(&mut reader),
    vec![
      Event {
        time: 0,
        handle: one,
        value: b"0 6 7".to_vec(),
        variable_length: false
      },
      Event {
        time: 0,
        handle: two,
        value: b"01 66 77".to_vec(),
        variable_length: false
      },
    ]
  );
}

#[test]
fn invalid_definitions_and_width_limits_preserve_the_variable_table() {
  let dir = TestDir::new();
  let path = dir.path("invalid-definitions.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  for (ty, direction, width, expected) in [
    (
      var_type::MAX + 1,
      var_dir::OUTPUT,
      1,
      Error::InvalidVariableDefinition(VariableDefinitionError::InvalidType(var_type::MAX + 1)),
    ),
    (
      var_type::VCD_REG,
      var_dir::MAX + 1,
      1,
      Error::InvalidVariableDefinition(VariableDefinitionError::InvalidDirection(var_dir::MAX + 1)),
    ),
    (
      var_type::VCD_REG,
      var_dir::OUTPUT,
      i32::MAX as u32 + 1,
      Error::LimitExceeded(
        LimitKind::VariableWidth,
        i32::MAX as u64 + 1,
        i32::MAX as u64,
      ),
    ),
  ] {
    assert_eq!(
      writer.create_var(ty, direction, width, "rejected", None),
      Err(expected)
    );
  }
  let bit = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
    .unwrap();
  assert_eq!(u32::from(bit), 1);
  writer.emit_value_change(bit, b"1").unwrap();
  writer.emit_time_change(1).unwrap();
  drop(writer);
  let mut reader = Reader::open(path).unwrap();
  assert_eq!(reader.var_count(), 1);
  reader.set_mask_all();
  assert_eq!(events(&mut reader)[0].value, b"1");
}

#[test]
fn implicit_time_zero_records_each_successful_value_change() {
  let dir = TestDir::new();
  let path = dir.path("time-zero.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let bit = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
    .unwrap();
  writer.emit_value_change(bit, b"0").unwrap();
  writer.emit_value_change(bit, b"1").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  reader.set_mask_all();
  let mut changes = Vec::new();
  reader
    .for_each_block(|time, _, value, _| changes.push((time, value.to_vec())))
    .unwrap();
  assert_eq!(changes, vec![(0, b"0".to_vec()), (0, b"1".to_vec())]);
}
#[test]
fn errors_preserve_their_standard_error_sources() {
  use std::error::Error as _;
  let cause = std::ffi::CString::new(b"a\0b".to_vec()).unwrap_err();
  let error = fstapi::Error::CStringConv(cause.clone());
  let boxed: Box<dyn std::error::Error> = error.into();
  assert!(boxed.downcast_ref::<fstapi::Error>().is_some());
  assert_eq!(
    boxed.source().unwrap().downcast_ref::<std::ffi::NulError>(),
    Some(&cause)
  );
  assert!(fstapi::Error::ContextCreate.source().is_none());
}
