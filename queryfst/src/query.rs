use crate::Cli;
use fstapi::{Handle, Reader, VarType, var_type};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::io::Write;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// One physical facility and its hierarchy names, in handle order.
struct Signal {
  handle: Handle,
  path: String,
  aliases: Vec<String>,
  width: u32,
  ty: VarType,
}

impl Signal {
  /// Identifies variable-length values, whose block frames have no initial state.
  fn variable(&self) -> bool {
    self.width == 0 || self.ty == var_type::GEN_STRING
  }

  /// Selects a lossless JSON representation without narrowing bit vectors.
  fn encoding(&self) -> &'static str {
    match self.ty {
      _ if self.variable() => "bytes_hex",
      var_type::VCD_REAL
      | var_type::VCD_REAL_PARAMETER
      | var_type::VCD_REALTIME
      | var_type::SV_SHORTREAL => "real_f64_le_hex",
      var_type::VCD_PORT => "evcd",
      _ => "bits",
    }
  }

  /// Encodes variable bytes in hex and textual callbacks without lossy UTF-8.
  fn value(&self, bytes: &[u8]) -> Result<String> {
    let real_bytes;
    let bytes = if self.encoding() == "real_f64_le_hex" {
      real_bytes = u64::from_ne_bytes(bytes.try_into()?).to_le_bytes();
      &real_bytes
    } else {
      bytes
    };
    if self.variable() || self.encoding() == "real_f64_le_hex" {
      const HEX: &[u8] = b"0123456789abcdef";
      let mut result = String::with_capacity(bytes.len() * 2);
      for &byte in bytes {
        result.push(HEX[usize::from(byte >> 4)] as char);
        result.push(HEX[usize::from(byte & 15)] as char);
      }
      Ok(result)
    } else {
      Ok(std::str::from_utf8(bytes)?.to_owned())
    }
  }

  /// Writes an observed value or explicit unavailable initial state.
  fn record(
    &self,
    kind: &str,
    time: u64,
    value: Option<&[u8]>,
    source_time: Option<u64>,
  ) -> Result<Value> {
    Ok(json!({
      "type": kind, "time": time.to_string(), "handle": u32::from(self.handle),
      "encoding": self.encoding(), "value": value.map(|value| self.value(value)).transpose()?,
      "source_time": source_time.map(|time| time.to_string()),
    }))
  }
}

/// Per-facility state; no event history is retained.
struct State {
  previous: Option<Vec<u8>>,
  source_time: Option<u64>,
  accounted_until: u64,
  residency: [u64; 6],
  transitions: u64,
  callbacks: u64,
}

impl State {
  /// Starts a signal with unknown state at the requested interval boundary.
  fn new(start: u64) -> Self {
    Self {
      previous: None,
      source_time: None,
      accounted_until: start,
      residency: [0; 6],
      transitions: 0,
      callbacks: 0,
    }
  }

  /// Integrates state over elapsed ticks, with no duration assigned at the end.
  fn account(&mut self, time: u64) {
    let index = match self.previous.as_deref() {
      Some(b"0") => 0,
      Some(b"1") => 1,
      Some(b"x" | b"X") => 2,
      Some(b"z" | b"Z") => 3,
      Some(_) => 4,
      None => 5,
    };
    self.residency[index] += time - self.accounted_until;
    self.accounted_until = time;
  }
}

/// Emits one complete JSON record per line and propagates output failures.
fn line(output: &mut impl Write, record: &Value) -> Result<()> {
  serde_json::to_writer(&mut *output, record)?;
  output.write_all(b"\n")?;
  Ok(())
}

/// Selects handles first, then collects canonical paths and every alias.
fn select(reader: &mut Reader, cli: &Cli) -> Result<Vec<Signal>> {
  let regex = cli.signals.as_deref().map(Regex::new).transpose()?;
  let mut missing: HashSet<&str> = cli.signal.iter().map(String::as_str).collect();
  let exact = missing.clone();
  let mut handles = HashSet::new();
  for entry in reader.vars() {
    let (path, var) = entry.map_err(|error| error.to_string())?;
    if exact.contains(path.as_str()) || regex.as_ref().is_some_and(|re| re.is_match(&path)) {
      handles.insert(var.handle());
      missing.remove(path.as_str());
    }
  }
  if !missing.is_empty() {
    let mut missing: Vec<_> = missing.into_iter().collect();
    missing.sort_unstable();
    return Err(format!("exact signal paths not found: {}", missing.join(", ")).into());
  }
  if handles.is_empty() {
    return Err("no signals matched the selection".into());
  }
  let mut signals: BTreeMap<Handle, Signal> = BTreeMap::new();
  for entry in reader.vars() {
    let (path, var) = entry.map_err(|error| error.to_string())?;
    if !handles.contains(&var.handle()) {
      continue;
    }
    if var.is_alias() {
      signals
        .get_mut(&var.handle())
        .ok_or("alias precedes its physical facility")?
        .aliases
        .push(path);
    } else {
      signals.insert(
        var.handle(),
        Signal {
          handle: var.handle(),
          path,
          aliases: Vec::new(),
          width: var.length(),
          ty: var.ty(),
        },
      );
    }
  }
  Ok(signals.into_values().collect())
}

/// Reports any recording gap intersecting an elapsed-time interval [start, end).
fn recording_gap(activity: &[(u64, bool)], start: u64, end: u64) -> bool {
  let mut active = true;
  let mut previous = start;
  for &(time, next) in activity {
    if time <= start {
      active = next;
    } else if time < end {
      if !active && time > previous {
        return true;
      }
      active = next;
      previous = time;
    } else {
      break;
    }
  }
  !active && end > previous
}

/// Includes zero-duration off/on pairs, which can hide same-timestamp changes.
fn recording_interruption(activity: &[(u64, bool)], start: u64, end: u64) -> bool {
  recording_gap(activity, start, end)
    || activity
      .iter()
      .any(|&(time, active)| !active && start <= time && time <= end)
}

/// Emits state immediately before start, separately from all callbacks at start.
fn initial(
  output: &mut impl Write,
  signals: &[Signal],
  states: &mut [State],
  activity: &[(u64, bool)],
  start: u64,
) -> Result<()> {
  for (signal, state) in signals.iter().zip(states) {
    if state
      .source_time
      .is_some_and(|time| recording_interruption(activity, time, start))
    {
      state.previous = None;
      state.source_time = None;
    }
    line(
      output,
      &signal.record(
        "initial",
        start,
        state.previous.as_deref(),
        state.source_time,
      )?,
    )?;
  }
  Ok(())
}

/// Runs a masked streaming query with explicit boundary and truncation records.
pub(super) fn run(cli: Cli, output: &mut impl Write) -> Result<()> {
  let mut reader = Reader::open(&cli.input).map_err(|error| error.to_string())?;
  let start = cli.start.unwrap_or(reader.start_time());
  let end = cli.end.unwrap_or(reader.end_time());
  if start > end || start < reader.start_time() || end > reader.end_time() {
    return Err(
      format!(
        "range must satisfy {} <= start <= end <= {} (raw FST ticks)",
        reader.start_time(),
        reader.end_time()
      )
      .into(),
    );
  }
  let signals = select(&mut reader, &cli)?;
  if cli.summary
    && signals.iter().any(|signal| {
      signal.width != 1 || signal.encoding() != "bits" || signal.ty == var_type::VCD_EVENT
    })
  {
    return Err("--summary requires scalar state signals (one-bit logic, excluding events)".into());
  }
  let activity = reader.dump_activity();
  if cli.summary && recording_interruption(&activity, start, end) {
    return Err(
      "--summary cannot infer residency across inactive waveform-dumping intervals".into(),
    );
  }
  // Variable-length records have no frame snapshots. A snapshot after a past
  // dumping gap can also retain stale values, hiding their prior observation.
  // Scan from the file start in these cases to recover trustworthy provenance.
  let scan_start = if signals.iter().any(Signal::variable)
    || activity
      .iter()
      .any(|&(time, active)| !active && time < start)
  {
    reader.start_time()
  } else {
    start
  };
  line(
    output,
    &json!({
      "type": "header", "schema": "queryfst", "schema_version": 1,
      "start": start.to_string(), "end": end.to_string(),
      "trace_start": reader.start_time().to_string(), "trace_end": reader.end_time().to_string(),
      "timescale_exponent": reader.timescale(), "timezero": reader.timezero().to_string(),
      "interval": "inclusive", "initial_semantics": "latest_callback_strictly_before_start",
      "event_order": "libfst_callback_order", "limit": cli.limit.to_string(),
      "mode": if cli.summary { "scalar_summary" } else { "events" },
      "scan_start": scan_start.to_string(), "early_termination": false,
    }),
  )?;
  for signal in &signals {
    line(
      output,
      &json!({
        "type": "signal", "handle": u32::from(signal.handle), "path": signal.path,
        "aliases": signal.aliases, "width": signal.width, "var_type": signal.ty,
        "encoding": signal.encoding(),
      }),
    )?;
  }
  for &(time, active) in &activity {
    line(
      output,
      &json!({"type": "dump_activity", "time": time.to_string(), "active": active}),
    )?;
  }

  reader.clear_mask_all();
  for signal in &signals {
    reader.set_mask(signal.handle);
  }
  reader.set_time_range_limit(scan_start, end);
  // Preserve IEEE-754 bits, including NaN payloads, rather than asking libfst
  // to format a double with a potentially lossy decimal precision.
  reader.set_native_doubles_on_callback(true);
  let indices: HashMap<_, _> = signals
    .iter()
    .enumerate()
    .map(|(index, signal)| (signal.handle, index))
    .collect();
  let mut states: Vec<_> = signals.iter().map(|_| State::new(start)).collect();
  let mut initialized = false;
  let mut callbacks = 0u64;
  let mut matched = 0u64;
  let mut emitted = 0u64;
  let mut callback_error = None;
  reader
    .for_each_block(|time, handle, value, _| {
      if callback_error.is_some() {
        return;
      }
      let result = (|| -> Result<()> {
        callbacks += 1;
        if !initialized && time >= start {
          initial(output, &signals, &mut states, &activity, start)?;
          initialized = true;
        }
        if time > end {
          return Ok(());
        }
        let index = indices[&handle];
        let state = &mut states[index];
        if time < start {
          state.previous = Some(value.to_vec());
          state.source_time = Some(time);
          return Ok(());
        }
        matched += 1;
        if cli.summary {
          state.account(time);
          if state.previous.as_deref().is_some_and(|old| old != value) {
            state.transitions += 1;
          }
          state.previous = Some(value.to_vec());
          state.callbacks += 1;
        } else if emitted < cli.limit {
          let mut record = signals[index].record("event", time, Some(value), None)?;
          record["sequence"] = emitted.to_string().into();
          line(output, &record)?;
          emitted += 1;
        }
        Ok(())
      })();
      if let Err(error) = result {
        callback_error = Some(error);
      }
    })
    .map_err(|error| error.to_string())?;
  if let Some(error) = callback_error {
    return Err(error);
  }
  if !initialized {
    initial(output, &signals, &mut states, &activity, start)?;
  }
  if cli.summary {
    for (signal, state) in signals.iter().zip(&mut states) {
      state.account(end);
      let residency: BTreeMap<_, _> = ["0", "1", "x", "z", "other", "unavailable"]
        .into_iter()
        .zip(state.residency.map(|duration| duration.to_string()))
        .collect();
      line(
        output,
        &json!({
          "type": "scalar_summary", "handle": u32::from(signal.handle),
          "duration_ticks": (end - start).to_string(), "residency_ticks": residency,
          "value_transitions": state.transitions.to_string(), "callbacks": state.callbacks.to_string(),
        }),
      )?;
    }
  }
  line(
    output,
    &json!({
      "type": "summary", "complete": true, "selected_handles": signals.len(),
      "decoded_callbacks": callbacks.to_string(), "matching_callbacks": matched.to_string(),
      "emitted_events": emitted.to_string(), "omitted_events": if cli.summary { "0".to_string() } else { (matched - emitted).to_string() },
      "truncated": !cli.summary && matched > emitted,
    }),
  )
}
