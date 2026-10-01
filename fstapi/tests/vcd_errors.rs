mod common;

use common::*;
use fstapi::{Error, Reader, writer_pack_type};
use std::fs;

#[test]
fn vcd_dump_preserves_hierarchy_values_and_reports_open_failure() {
  let dir = TestDir::new();
  let path = dir.path("dump.fst");
  let handles = fixture(&path, writer_pack_type::ZLIB, true, false);
  let mut reader = Reader::open(&path).unwrap();
  reader.clear_mask_all();
  let vcd_path = dir.path("dump.vcd");
  reader.dump_as_vcd(Some(&vcd_path)).unwrap();
  assert!(handles.iter().all(|&handle| reader.mask(handle)));
  let vcd = fs::read_to_string(vcd_path).unwrap();
  for expected in [
    "$scope module top $end",
    "$scope module child $end",
    "vector $end",
    "alias $end",
    "$enddefinitions $end",
    "#0\n",
    "#10\n",
    "#20\n",
    "b10001000 ",
    "b00111001 ",
    "r1.25 ",
    "r-2.5 ",
    "p01 66 77 ",
  ] {
    assert!(vcd.contains(expected), "missing {expected:?} in {vcd}");
  }
  assert_eq!(
    reader.dump_as_vcd(Some(dir.path("missing/dump.vcd"))),
    Err(Error::InvalidOperation)
  );
}

#[cfg(target_os = "linux")]
#[test]
fn vcd_dump_reports_flush_failures_and_reader_can_be_reused() {
  let dir = TestDir::new();
  let path = dir.path("full-device.fst");
  fixture(&path, writer_pack_type::LZ4, true, false);
  let mut reader = Reader::open(&path).unwrap();
  // A small export remains buffered until fclose, where /dev/full fails.
  assert_eq!(
    reader.dump_as_vcd(Some("/dev/full")),
    Err(Error::InvalidOperation)
  );
  let output = dir.path("after-error.vcd");
  reader.dump_as_vcd(Some(&output)).unwrap();
  let vcd = fs::read_to_string(output).unwrap();
  assert!(vcd.contains("b10001000 "));
  assert!(vcd.contains("#10\n"));
}
