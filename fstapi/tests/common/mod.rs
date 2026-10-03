#![allow(dead_code)]

use fstapi::{Handle, Reader, Writer, file_type, scope_type, var_dir, var_type};
use std::path::Path;

mod temp_dir;
pub use temp_dir::TestDir;

/// An owned value-change event that can be sorted for deterministic comparisons.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
  /// Timestamp in the waveform's time units.
  pub time: u64,
  /// Handle of the signal whose value changed.
  pub handle: Handle,
  /// Bytes copied from the reader callback.
  pub value: Vec<u8>,
  /// Whether the value came from the variable-length callback.
  pub variable_length: bool,
}

/// Collects and sorts events using the reader's current masks and time limits.
pub fn events(reader: &mut Reader) -> Vec<Event> {
  let mut events = Vec::new();
  reader
    .for_each_block(|time, handle, value, variable_length| {
      events.push(Event {
        time,
        handle,
        value: value.to_vec(),
        variable_length,
      });
    })
    .unwrap();
  events.sort();
  events
}

/// Writes a round-trip fixture using the requested compression and repack options.
///
/// Returns the vector, real, string, and EVCD port handles, in that order.
/// The fixture also includes nested scopes and an alias of the vector.
pub fn fixture(
  path: &Path,
  pack: fstapi::WriterPackType,
  compressed_hier: bool,
  repack: bool,
) -> [Handle; 4] {
  let mut writer = Writer::create(path, compressed_hier)
    .unwrap()
    .pack_type(pack)
    .parallel_mode(repack)
    .repack_on_close(repack)
    .timescale_from_str("1ns")
    .unwrap()
    .timezero(-7)
    .file_type(file_type::VERILOG)
    .date("regression fixture")
    .unwrap()
    .version("fstapi regression")
    .unwrap()
    .comment("round-trip fixture")
    .unwrap();
  writer
    .set_scope(scope_type::VCD_MODULE, "top", "top")
    .unwrap();
  let vector = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "vector", None)
    .unwrap();
  let real = writer
    .create_var(var_type::VCD_REAL, var_dir::OUTPUT, 64, "real", None)
    .unwrap();
  // The writer normalizes string width to zero and real width to eight bytes.
  let string = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 32, "string", None)
    .unwrap();
  // EVCD stores the value and two strength vectors: 3 * 2 + 2 bytes.
  let port = writer
    .create_var(var_type::VCD_PORT, var_dir::INOUT, 8, "port", None)
    .unwrap();
  writer
    .set_scope(scope_type::VCD_MODULE, "child", "child")
    .unwrap();
  let alias = writer
    .create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "alias", Some(vector))
    .unwrap();
  assert_eq!(alias, vector);
  writer.set_upscope();
  writer.set_upscope();

  for (time, vector_value, real_value, string_value, port_value) in [
    (0, b"10001000", 1.25_f64, b"a\0b", b"01 66 77"),
    (10, b"00111001", -2.5_f64, b"c\0d", b"10 77 66"),
  ] {
    writer.emit_time_change(time).unwrap();
    writer.emit_value_change(alias, vector_value).unwrap();
    writer
      .emit_value_change(real, &real_value.to_ne_bytes())
      .unwrap();
    writer
      .emit_var_len_value_change(string, string_value)
      .unwrap();
    writer.emit_value_change(port, port_value).unwrap();
  }
  writer.emit_time_change(20).unwrap();
  drop(writer);
  [vector, real, string, port]
}

/// Returns sorted fixture events with real values encoded for the callback mode.
///
/// Handles must be in the order returned by [`fixture`].
pub fn expected_events(handles: [Handle; 4], native_doubles: bool) -> Vec<Event> {
  let mut expected = Vec::new();
  for (time, vector, real, string, port) in [
    (0, b"10001000", 1.25_f64, b"a\0b", b"01 66 77"),
    (10, b"00111001", -2.5_f64, b"c\0d", b"10 77 66"),
  ] {
    for (index, value) in [
      vector.to_vec(),
      if native_doubles {
        real.to_ne_bytes().to_vec()
      } else {
        real.to_string().into_bytes()
      },
      string.to_vec(),
      port.to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
      expected.push(Event {
        time,
        handle: handles[index],
        value,
        variable_length: index == 2,
      });
    }
  }
  expected.sort();
  expected
}
