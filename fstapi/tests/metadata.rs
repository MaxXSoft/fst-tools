mod common;

use common::TestDir;
use fstapi::{Hier, Reader, Writer, attr_type, misc_type, var_dir, var_type};
use std::ffi::CString;

#[test]
fn binary_source_attributes_round_trip_without_utf8_conversion() {
  let dir = TestDir::new();
  let path = dir.path("binary-attributes.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  for (subtype, index) in [(misc_type::SOURCESTEM, 128), (misc_type::SOURCEISTEM, 129)] {
    let name = CString::new(vec![index as u8, 1]).unwrap();
    writer
      .set_attr_begin_raw(attr_type::MISC, subtype as _, &name, 42)
      .unwrap();
  }
  let handle = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "signal", None)
    .unwrap();
  writer.emit_value_change(handle, b"1").unwrap();
  writer.emit_time_change(10).unwrap();
  drop(writer);

  let mut reader = Reader::open(&path).unwrap();
  let attributes: Vec<_> = reader
    .hiers()
    .filter_map(|hier| match hier {
      Hier::AttrBegin(attr) => Some(attr),
      _ => None,
    })
    .collect();
  assert_eq!(attributes.len(), 2);
  for (attr, index) in attributes.iter().zip([128, 129]) {
    assert_eq!(attr.name_cstr().to_bytes(), &[index as u8, 1]);
    assert_eq!(attr.arg_from_name(), index);
    assert_eq!(attr.arg(), 42);
    assert!(attr.name().is_err());
  }
}

#[test]
fn dump_activity_retains_all_transitions_independent_of_reader_filters() {
  let dir = TestDir::new();
  let path = dir.path("activity.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let handle = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "signal", None)
    .unwrap();
  writer.emit_dump_active(false).unwrap();
  writer.emit_value_change(handle, b"0").unwrap();
  writer.emit_time_change(5).unwrap();
  writer.emit_dump_active(true).unwrap();
  writer.emit_dump_active(false).unwrap();
  writer.emit_time_change(10).unwrap();
  writer.emit_dump_active(true).unwrap();
  drop(writer);

  let mut reader = Reader::open(&path).unwrap();
  reader.clear_mask_all();
  reader.set_time_range_limit(6, 8);
  assert_eq!(
    reader.dump_activity(),
    vec![(0, false), (5, true), (5, false), (10, true)]
  );
}
