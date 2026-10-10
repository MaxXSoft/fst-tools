mod common;

use common::*;
use fstapi::{Reader, Writer, file_type, var_dir, var_type, writer_pack_type};

#[test]
fn round_trip_compression_hierarchy_and_repack() {
  for pack in [
    writer_pack_type::ZLIB,
    writer_pack_type::FASTLZ,
    writer_pack_type::LZ4,
  ] {
    for compressed_hier in [false, true] {
      for repack in [false, true] {
        let dir = TestDir::new();
        let path = dir.path("fixture.fst");
        let case = format!("pack {pack}, compressed_hier {compressed_hier}, repack {repack}");
        let mut expected_files = vec![path.clone()];
        if !compressed_hier {
          expected_files.push(dir.path("fixture.fst.hier"));
        }
        expected_files.sort();
        let assert_files = |stage| {
          let mut files: Vec<_> = std::fs::read_dir(dir.path(""))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
          files.sort();
          assert_eq!(files, expected_files, "{case}, after {stage}");
        };
        let handles = fixture(&path, pack, compressed_hier, repack);
        assert_files("writer close");
        let mut reader = Reader::open(&path).unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(reader.date().unwrap(), "regression fixture", "{case}");
        assert_eq!(reader.version().unwrap(), "fstapi regression", "{case}");
        assert_eq!((reader.start_time(), reader.end_time()), (0, 20), "{case}");
        assert_eq!(reader.file_type(), file_type::VERILOG, "{case}");
        assert_eq!(
          (
            reader.scope_count(),
            reader.var_count(),
            reader.alias_count()
          ),
          (2, 5, 1),
          "{case}"
        );
        assert_eq!(
          (reader.timescale(), reader.timescale_str()),
          (-9, Some("1ns")),
          "{case}"
        );
        assert_eq!(reader.timezero(), -7, "{case}");
        let variables: Vec<_> = reader
          .vars()
          .map(|entry| {
            let (name, var) = entry.unwrap_or_else(|error| panic!("{case}: {error}"));
            (
              name,
              var.ty(),
              var.direction(),
              var.length(),
              var.handle(),
              var.is_alias(),
            )
          })
          .collect();
        assert_eq!(
          variables,
          vec![
            (
              "top.vector".into(),
              var_type::VCD_REG,
              var_dir::OUTPUT,
              8,
              handles[0],
              false
            ),
            (
              "top.real".into(),
              var_type::VCD_REAL,
              var_dir::OUTPUT,
              8,
              handles[1],
              false
            ),
            (
              "top.string".into(),
              var_type::GEN_STRING,
              var_dir::OUTPUT,
              0,
              handles[2],
              false
            ),
            (
              "top.port".into(),
              var_type::VCD_PORT,
              var_dir::INOUT,
              2,
              handles[3],
              false
            ),
            (
              "top.child.alias".into(),
              var_type::VCD_REG,
              var_dir::OUTPUT,
              8,
              handles[0],
              true
            ),
          ],
          "{case}"
        );
        reader.set_mask_all();
        for native_doubles in [false, true, false] {
          reader.set_native_doubles_on_callback(native_doubles);
          assert_eq!(
            events(&mut reader),
            expected_events(handles, native_doubles),
            "{case}, native_doubles {native_doubles}"
          );
        }
        drop(reader);
        assert_files("reader close");
      }
    }
  }
}

#[test]
fn concurrent_writers_keep_scratch_files_isolated() {
  let start = std::sync::Barrier::new(8);
  std::thread::scope(|scope| {
    for worker in 0..8 {
      let start = &start;
      scope.spawn(move || {
        let dir = TestDir::new();
        let path = dir.path("concurrent.fst");
        start.wait();
        for iteration in 0..32 {
          let value = format!("{:08b}", worker * 32 + iteration).into_bytes();
          let mut writer = Writer::create(&path, true).unwrap();
          let handle = writer
            .create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "value", None)
            .unwrap();
          writer.emit_time_change(0).unwrap();
          writer.emit_value_change(handle, &value).unwrap();
          writer.emit_time_change(1).unwrap();
          drop(writer);

          let mut reader = Reader::open(&path).unwrap();
          assert_eq!(reader.vars().next().unwrap().unwrap().0, "value");
          reader.set_mask_all();
          assert_eq!(
            events(&mut reader),
            vec![Event {
              time: 0,
              handle,
              value,
              variable_length: false,
            }],
            "worker {worker}, iteration {iteration}"
          );
        }
      });
    }
  });
}

#[test]
fn reading_values_does_not_require_a_prior_hierarchy_walk() {
  let dir = TestDir::new();
  let path = dir.path("no-hierarchy-walk.fst");
  let handles = fixture(&path, writer_pack_type::LZ4, true, false);
  let mut reader = Reader::open(&path).unwrap();
  reader.set_mask_all();
  assert_eq!(events(&mut reader), expected_events(handles, false));
}

#[test]
fn process_masks_and_time_range_can_be_reset() {
  let dir = TestDir::new();
  let path = dir.path("masked.fst");
  let handles = fixture(&path, writer_pack_type::LZ4, true, false);
  let mut reader = Reader::open(&path).unwrap();
  reader.clear_mask_all();
  assert!(handles.iter().all(|&handle| !reader.mask(handle)));
  assert!(events(&mut reader).is_empty());
  reader.set_mask(handles[0]);
  reader.set_mask(handles[2]);
  reader.clear_mask(handles[2]);
  assert!(reader.mask(handles[0]));
  assert!(!reader.mask(handles[2]));
  let expected = expected_events(handles, false);
  assert_eq!(
    events(&mut reader),
    expected_events(handles, false)
      .into_iter()
      .filter(|event| event.handle == handles[0])
      .collect::<Vec<_>>()
  );
  // libfst uses this range to skip whole blocks in callback mode.
  reader.set_time_range_limit(21, 30);
  assert!(events(&mut reader).is_empty());
  reader.reset_time_range_limit();
  reader.set_mask_all();
  assert!(handles.iter().all(|&handle| reader.mask(handle)));
  assert_eq!(events(&mut reader), expected);
}

#[test]
fn all_real_types_use_eight_byte_values() {
  let dir = TestDir::new();
  let path = dir.path("real-types.fst");
  let mut writer = Writer::create(&path, true).unwrap();
  let mut handles = Vec::new();
  for ty in [
    var_type::VCD_REAL,
    var_type::VCD_REAL_PARAMETER,
    var_type::VCD_REALTIME,
    var_type::SV_SHORTREAL,
  ] {
    let handle = writer
      .create_var(ty, var_dir::OUTPUT, 1, &format!("real{ty}"), None)
      .unwrap();
    handles.push(handle);
  }
  writer.emit_time_change(0).unwrap();
  for &handle in &handles {
    writer
      .emit_value_change(handle, &0.125_f64.to_ne_bytes())
      .unwrap();
  }
  writer.emit_time_change(10).unwrap();
  drop(writer);
  let mut reader = Reader::open(&path).unwrap();
  reader.set_mask_all();
  for native in [true, false] {
    reader.set_native_doubles_on_callback(native);
    let actual = events(&mut reader);
    assert_eq!(actual.len(), handles.len());
    for (event, handle) in actual.iter().zip(&handles) {
      assert_eq!(event.handle, *handle);
      assert_eq!(event.time, 0);
      assert!(!event.variable_length);
      assert_eq!(
        event.value,
        if native {
          0.125_f64.to_ne_bytes().to_vec()
        } else {
          b"0.125".to_vec()
        }
      );
    }
  }
}
