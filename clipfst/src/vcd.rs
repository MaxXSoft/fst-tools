use crate::try_or_exit;
use fstapi::{Handle, Reader, Result, Writer};
use std::collections::HashMap;
use std::mem;

/// Value change data writer.
pub struct VcdWriter {
  writer: Writer,
  start_time: u64,
  end_time: u64,
  handles: HashMap<Handle, Handle>,
  last_time: u64,
  last_values: HashMap<Handle, Box<[u8]>>,
}

impl VcdWriter {
  pub fn new(writer: Writer, start: u64, end: u64, handles: HashMap<Handle, Handle>) -> Self {
    Self {
      writer,
      start_time: start,
      end_time: end,
      handles,
      last_time: start,
      last_values: HashMap::new(),
    }
  }

  pub fn write(&mut self, reader: &mut Reader) -> Result<()> {
    reader.for_each_block(|time, handle, value, var_len| {
      // Check time range.
      if !self.is_in_time_range(time, handle, value) {
        return;
      }
      // Write previous value changes.
      self.write_prev_value_changes();
      // Write time change.
      self.write_time_change(time);
      // Write value change.
      if var_len {
        self.write_var_len_value_change(handle, value)
      } else {
        self.write_value_change(handle, value);
      }
    })?;
    self.write_time_change(self.end_time);
    Ok(())
  }

  fn is_in_time_range(&mut self, time: u64, handle: Handle, value: &[u8]) -> bool {
    if time < self.start_time {
      // Record previous value changes.
      self.last_values.insert(handle, value.into());
      false
    } else if time > self.end_time {
      false
    } else {
      true
    }
  }

  fn write_prev_value_changes(&mut self) {
    if !self.last_values.is_empty() {
      for (handle, value) in mem::take(&mut self.last_values) {
        self.write_value_change(handle, &value)
      }
    }
  }

  fn write_time_change(&mut self, time: u64) {
    if time != self.last_time {
      let ret = self.writer.emit_time_change(time - self.start_time);
      try_or_exit!(ret, _, "Failed to write time change!");
      self.last_time = time;
    }
  }

  fn write_value_change(&mut self, handle: Handle, value: &[u8]) {
    let ret = self.writer.emit_value_change(self.handles[&handle], value);
    try_or_exit!(ret, _, "Failed to write value change!");
  }

  fn write_var_len_value_change(&mut self, handle: Handle, value: &[u8]) {
    let handle = self.handles[&handle];
    let ret = self.writer.emit_var_len_value_change(handle, value);
    try_or_exit!(ret, _, "Failed to write value change!");
  }
}
