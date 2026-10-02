use crate::hiers::Selection;
use fstapi::{Handle, Reader, Result, Writer};
use std::collections::{HashMap, VecDeque};
use std::mem;

/// Value change data writer.
pub struct VcdWriter {
  writer: Writer,
  start_time: u64,
  end_time: u64,
  handles: HashMap<Handle, Handle>,
  has_variable_values: bool,
  dump_activity: VecDeque<(u64, bool)>,
  last_time: u64,
  last_values: HashMap<Handle, (Box<[u8]>, bool)>,
  #[cfg(test)]
  visited_changes: usize,
}

impl VcdWriter {
  pub fn new(writer: Writer, start: u64, end: u64, selection: Selection) -> Self {
    Self {
      writer,
      start_time: start,
      end_time: end,
      handles: selection.handles,
      has_variable_values: selection.has_variable_values,
      dump_activity: VecDeque::new(),
      last_time: start,
      last_values: HashMap::new(),
      #[cfg(test)]
      visited_changes: 0,
    }
  }

  pub fn write(&mut self, reader: &mut Reader) -> Result<()> {
    // Fixed-width facilities have a snapshot at each block boundary. Include
    // the preceding tick so boundary events remain distinct from that snapshot.
    // Variable-length facilities have no snapshots and need earlier history.
    let mut lookback = 1u64;
    let mut read_start = if self.has_variable_values {
      reader.start_time()
    } else {
      self.start_time.saturating_sub(1).max(reader.start_time())
    };
    let mut active = true;
    for (time, enable) in reader.dump_activity() {
      if time < self.start_time {
        active = enable;
      } else if time <= self.end_time {
        self.dump_activity.push_back((time, enable));
      }
    }
    if !active
      && self
        .dump_activity
        .front()
        .is_none_or(|&(time, _)| time > self.start_time)
    {
      self.dump_activity.push_front((self.start_time, false));
    }
    // The writer accepts real values as native doubles, and variable-length
    // values need an explicit time before their first emission.
    reader.set_native_doubles_on_callback(true);
    self.writer.emit_time_change(0)?;
    loop {
      reader.set_time_range_limit(read_start, self.end_time);
      let mut result = Ok(());
      let mut saw_value = false;
      reader.for_each_block(|time, handle, value, var_len| {
        saw_value = true;
        if result.is_ok() {
          result = self.write_change(time, handle, value, var_len);
        }
      })?;
      result?;
      if saw_value || read_start == reader.start_time() {
        break;
      }
      // The file's end time can extend past its last value-change block. Walk
      // back until one supplies an initial snapshot, without replaying the
      // entire history of a long waveform for a quiet trailing window.
      lookback = lookback.saturating_mul(2);
      read_start = self
        .start_time
        .saturating_sub(lookback)
        .max(reader.start_time());
    }
    // This also handles windows in which none of the selected signals change.
    self.write_prev_value_changes()?;
    self.write_dump_activity(self.end_time)?;
    self.write_time_change(self.end_time)
  }

  fn write_change(&mut self, time: u64, handle: Handle, value: &[u8], var_len: bool) -> Result<()> {
    #[cfg(test)]
    {
      self.visited_changes += 1;
    }
    if time < self.start_time {
      self.last_values.insert(handle, (value.into(), var_len));
      return Ok(());
    }
    if time > self.end_time {
      return Ok(());
    }
    if time == self.start_time {
      // Every event at the boundary belongs to the clip. Its first value
      // supplies this facility's initial state, replacing the older snapshot.
      self.last_values.remove(&handle);
    } else {
      self.write_prev_value_changes()?;
    }
    self.write_dump_activity(time)?;
    self.write_time_change(time)?;
    self.write_value_change(handle, value, var_len)
  }

  fn write_dump_activity(&mut self, through: u64) -> Result<()> {
    while let Some(&(time, active)) = self.dump_activity.front() {
      if time > through {
        break;
      }
      self.dump_activity.pop_front();
      self.write_time_change(time)?;
      self.writer.emit_dump_active(active)?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::hiers;
  use fstapi::{var_dir, var_type};
  use regex::Regex;

  #[test]
  fn fixed_width_windows_skip_earlier_blocks_and_keep_boundary_values() {
    let test_binary = std::env::current_exe().unwrap();
    let dir = tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap();
    let input = dir.path().join("input.fst");
    let mut writer = Writer::create(&input, true).unwrap();
    let signal = writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "signal", None)
      .unwrap();
    let string = writer
      .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "string", None)
      .unwrap();
    for time in 0..=20_000 {
      writer.emit_time_change(time).unwrap();
      writer
        .emit_value_change(signal, if time % 2 == 0 { b"0" } else { b"1" })
        .unwrap();
      if time == 0 {
        writer
          .emit_var_len_value_change(string, b"initial")
          .unwrap();
      }
      if time > 0 && time % 1_000 == 0 {
        writer.flush();
      }
    }
    writer.emit_time_change(20_001).unwrap();
    writer.emit_time_change(20_010).unwrap();
    drop(writer);

    // Exercise the file start, a new block's start and next tick, and a final
    // block containing only time changes. Each point clip must retain its value.
    let signal_pattern = Regex::new("^signal$").unwrap();
    for (start, end) in [
      (0, 0),
      (19_000, 19_000),
      (19_001, 19_001),
      (19_999, 19_999),
      (20_005, 20_005),
      (20_005, 20_010),
    ] {
      let mut reader = Reader::open(&input).unwrap();
      let output = dir.path().join(format!("window-{start}-{end}.fst"));
      let mut writer = Writer::create(&output, true).unwrap();
      let selection = hiers::build(
        &mut reader,
        &mut writer,
        Some(signal_pattern.clone()),
        false,
      )
      .unwrap();
      reader.clear_mask_all();
      for handle in selection.handles.keys() {
        reader.set_mask(*handle);
      }
      let mut clip = VcdWriter::new(writer, start, end, selection);
      clip.write(&mut reader).unwrap();
      assert!(
        clip.visited_changes <= 2_002,
        "start {start}: replayed {} changes for a short window",
        clip.visited_changes
      );
      drop(clip);
      let mut reader = Reader::open(output).unwrap();
      assert_eq!(reader.vars().count(), 1, "start {start}");
      assert_eq!(reader.end_time(), end - start);
      reader.set_mask_all();
      let mut values = Vec::new();
      reader
        .for_each_block(|time, _, value, _| values.push((time, value.to_vec())))
        .unwrap();
      assert_eq!(
        values,
        vec![(
          0,
          vec![if start < 20_000 && start % 2 == 1 {
            b'1'
          } else {
            b'0'
          }]
        )],
        "start {start}"
      );
    }
  }
}
