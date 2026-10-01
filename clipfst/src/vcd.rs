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
  last_values: HashMap<Handle, (Box<[u8]>, bool)>,
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
    // The writer accepts real values as native doubles, and variable-length
    // values need an explicit time before their first emission.
    reader.set_native_doubles_on_callback(true);
    self.writer.emit_time_change(0)?;
    let mut result = Ok(());
    reader.for_each_block(|time, handle, value, var_len| {
      if result.is_ok() {
        result = self.write_change(time, handle, value, var_len);
      }
    })?;
    result?;
    // This also handles windows in which none of the selected signals change.
    self.write_prev_value_changes()?;
    self.write_time_change(self.end_time)
  }

  fn write_change(&mut self, time: u64, handle: Handle, value: &[u8], var_len: bool) -> Result<()> {
    if time <= self.start_time {
      // Values at the boundary replace earlier values of the same signal.
      self.last_values.insert(handle, (value.into(), var_len));
      return Ok(());
    }
    if time > self.end_time {
      return Ok(());
    }
    self.write_prev_value_changes()?;
    self.write_time_change(time)?;
    self.write_value_change(handle, value, var_len)
  }

  fn write_prev_value_changes(&mut self) -> Result<()> {
    for (handle, (value, var_len)) in mem::take(&mut self.last_values) {
      self.write_value_change(handle, &value, var_len)?;
    }
    Ok(())
  }

  fn write_time_change(&mut self, time: u64) -> Result<()> {
    if time != self.last_time {
      self.writer.emit_time_change(time - self.start_time)?;
      self.last_time = time;
    }
    Ok(())
  }

  fn write_value_change(&mut self, handle: Handle, value: &[u8], var_len: bool) -> Result<()> {
    let handle = self.handles[&handle];
    if var_len {
      self.writer.emit_var_len_value_change(handle, value)
    } else {
      self.writer.emit_value_change(handle, value)
    }
  }
}
