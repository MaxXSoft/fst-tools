mod common;

use common::*;
use fstapi::{Error, Reader, Writer, var_dir, var_type, writer_pack_type};
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

#[test]
fn real_alias_capacity_boundaries_are_rejected_before_touching_output() {
  let dir = TestDir::new();
  for (unique_count, real_type, safe) in [
    (65535, var_type::VCD_REAL, true),
    (65536, var_type::VCD_REAL, false),
    (65537, var_type::VCD_REAL, true),
    (131072, var_type::VCD_REAL, false),
    (65536, var_type::VCD_REAL_PARAMETER, false),
    (65536, var_type::VCD_REALTIME, false),
    (65536, var_type::SV_SHORTREAL, false),
  ] {
    let path = dir.path(&format!("alias-{unique_count}-{real_type}.fst"));
    let vcd_path = path.with_extension("vcd");
    let mut writer = Writer::create(&path, true).unwrap();
    let real = writer
      .create_var(real_type, var_dir::OUTPUT, 64, "real", None)
      .unwrap();
    for i in 1..unique_count {
      writer
        .create_var(
          var_type::VCD_WIRE,
          var_dir::OUTPUT,
          1,
          &format!("bit{i}"),
          None,
        )
        .unwrap();
    }
    writer
      .create_var(real_type, var_dir::OUTPUT, 64, "alias", Some(real))
      .unwrap();
    writer
      .emit_value_change(real, &1.25_f64.to_ne_bytes())
      .unwrap();
    writer.emit_time_change(10).unwrap();
    drop(writer);

    let mut reader = Reader::open(&path).unwrap();
    fs::write(&vcd_path, "existing output").unwrap();
    let result = reader.dump_as_vcd(Some(&vcd_path));
    if safe {
      result.unwrap();
      let vcd = fs::read_to_string(vcd_path).unwrap();
      assert!(vcd.contains(" alias $end"));
      assert!(vcd.contains("r1.25 "));
    } else {
      assert_eq!(result, Err(Error::InvalidOperation));
      assert_eq!(fs::read_to_string(vcd_path).unwrap(), "existing output");
    }
    // Preflight also leaves hierarchy and value iteration usable on failure.
    assert_eq!(reader.hiers().count(), unique_count + 1);
    reader.clear_mask_all();
    reader.set_mask(real);
    assert_eq!(events(&mut reader)[0].value, b"1.25");
  }
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
