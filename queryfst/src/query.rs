use crate::{Cli, Error, Result, output::Output};
use fstapi::{Handle, Reader, VarType, var_type};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::ops::ControlFlow;
use std::time::Instant;

#[derive(Clone, Copy)]
enum ScanStop {
  CallbackBudget,
  DurationBudget,
}

impl ScanStop {
  fn as_str(self) -> &'static str {
    match self {
      Self::CallbackBudget => "callback_budget_exhausted",
      Self::DurationBudget => "duration_budget_exhausted",
    }
  }
}

/// Callback representation; strings are used only at the output boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
  Bits,
  BytesHex,
  RealF64LeHex,
  Evcd,
}

impl Encoding {
  fn as_str(self) -> &'static str {
    match self {
      Self::Bits => "bits",
      Self::BytesHex => "bytes_hex",
      Self::RealF64LeHex => "real_f64_le_hex",
      Self::Evcd => "evcd",
    }
  }
}

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
  fn encoding(&self) -> Encoding {
    match self.ty {
      _ if self.variable() => Encoding::BytesHex,
      var_type::VCD_REAL
      | var_type::VCD_REAL_PARAMETER
      | var_type::VCD_REALTIME
      | var_type::SV_SHORTREAL => Encoding::RealF64LeHex,
      var_type::VCD_PORT => Encoding::Evcd,
      _ => Encoding::Bits,
    }
  }

  /// Encodes variable bytes in hex and textual callbacks without lossy UTF-8.
  fn value(&self, bytes: &[u8]) -> Result<String> {
    let real_bytes;
    let bytes = if self.encoding() == Encoding::RealF64LeHex {
      real_bytes = u64::from_ne_bytes(bytes.try_into()?).to_le_bytes();
      &real_bytes
    } else {
      bytes
    };
    if self.variable() || self.encoding() == Encoding::RealF64LeHex {
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
      "type": kind,
      "time": time.to_string(),
      "handle": u32::from(self.handle),
      "encoding": self.encoding().as_str(),
      "value": value.map(|value| self.value(value)).transpose()?,
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

/// Selects handles first, then collects canonical paths and every alias.
fn select(reader: &mut Reader, cli: &Cli) -> Result<Vec<Signal>> {
  let regex = cli.signals.as_ref();
  let mut missing: HashSet<&str> = cli.signal.iter().map(String::as_str).collect();
  let exact = missing.clone();
  let mut handles = HashSet::new();
  for entry in reader.vars() {
    let (path, var) = entry?;
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
    let (path, var) = entry?;
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
  output: &mut Output<impl Write>,
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
    output.record(&signal.record(
      "initial",
      start,
      state.previous.as_deref(),
      state.source_time,
    )?)?;
  }
  Ok(())
}

#[derive(Clone, Copy)]
struct Window {
  start: u64,
  end: u64,
  scan_start: u64,
}

/// Runs a masked streaming query with explicit boundary and truncation records.
pub(super) fn run(cli: Cli, output: &mut Output<impl Write>) -> Result<()> {
  let mut reader = Reader::open(&cli.input)?;
  let start = cli.start.unwrap_or(reader.start_time());
  let end = cli.end.unwrap_or(reader.end_time());
  if start > end || start < reader.start_time() || end > reader.end_time() {
    return Err(Error::Arguments(format!(
      "range must satisfy {} <= start <= end <= {} (raw FST ticks)",
      reader.start_time(),
      reader.end_time()
    )));
  }
  let signals = select(&mut reader, &cli)?;
  if cli.summary
    && signals.iter().any(|signal| {
      signal.width != 1 || signal.encoding() != Encoding::Bits || signal.ty == var_type::VCD_EVENT
    })
  {
    return Err(Error::Arguments(
      "--summary requires scalar state signals (one-bit logic, excluding events)".into(),
    ));
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
  let window = Window {
    start,
    end,
    scan_start,
  };
  write_header(&reader, &cli, &signals, window, output)?;
  scan(&mut reader, &cli, &signals, &activity, window, output)
}

fn write_header(
  reader: &Reader,
  cli: &Cli,
  signals: &[Signal],
  window: Window,
  output: &mut Output<impl Write>,
) -> Result<()> {
  let Window {
    start,
    end,
    scan_start,
  } = window;
  output.record(&json!({
    "type": "header",
    "schema": "queryfst",
    "schema_version": 2,
    "start": start.to_string(),
    "end": end.to_string(),
    "trace_start": reader.start_time().to_string(),
    "trace_end": reader.end_time().to_string(),
    "timescale_exponent": reader.timescale(),
    "timezero": reader.timezero().to_string(),
    "interval": "inclusive",
    "initial_semantics": "latest_callback_strictly_before_start",
    "event_order": "libfst_callback_order",
    "limit": cli.max_rows.to_string(),
    "mode": if cli.summary { "scalar_summary" } else { "events" },
    "scan_start": scan_start.to_string(),
    "early_termination": true,
  }))?;
  for signal in signals {
    output.record(&json!({
      "type": "signal",
      "handle": u32::from(signal.handle),
      "path": signal.path,
      "aliases": signal.aliases,
      "width": signal.width,
      "var_type": signal.ty,
      "encoding": signal.encoding().as_str(),
    }))?;
  }
  for &(time, active) in &reader.dump_activity() {
    output.record(&json!({
      "type": "dump_activity",
      "time": time.to_string(),
      "active": active,
    }))?;
  }
  Ok(())
}

fn scan(
  reader: &mut Reader,
  cli: &Cli,
  signals: &[Signal],
  activity: &[(u64, bool)],
  window: Window,
  output: &mut Output<impl Write>,
) -> Result<()> {
  let Window {
    start,
    end,
    scan_start,
  } = window;
  reader.clear_mask_all();
  for signal in signals {
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
  let started = Instant::now();
  let mut reason = None;
  let mut last_callback_time = None;
  let mut processed_through = None;
  let skip = cli.max_callbacks == Some(0) || cli.max_duration_ms == Some(0);
  if skip {
    reason = Some(if cli.max_callbacks == Some(0) {
      ScanStop::CallbackBudget
    } else {
      ScanStop::DurationBudget
    });
  }
  if !skip {
    reader.for_each_block_controlled(|time, handle, value, _| {
      if last_callback_time != Some(time) {
        processed_through = time.checked_sub(1).filter(|t| *t >= start);
      }
      last_callback_time = Some(time);
      if time > end {
        return ControlFlow::Break(());
      }
      if cli
        .max_duration_ms
        .is_some_and(|n| started.elapsed().as_millis() >= u128::from(n))
      {
        reason = Some(ScanStop::DurationBudget);
        return ControlFlow::Break(());
      }
      let result = (|| -> Result<()> {
        callbacks += 1;
        if !initialized && time >= start {
          initial(output, signals, &mut states, activity, start)?;
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
        } else if emitted < cli.max_rows && !output.truncated {
          let mut record = signals[index].record("event", time, Some(value), None)?;
          record["sequence"] = emitted.to_string().into();
          if output.record(&record)? {
            emitted += 1;
          }
        }
        Ok(())
      })();
      if let Err(error) = result {
        callback_error = Some(error);
        return ControlFlow::Break(());
      }
      if cli.max_callbacks.is_some_and(|n| callbacks >= n) {
        reason = Some(ScanStop::CallbackBudget);
        return ControlFlow::Break(());
      }
      ControlFlow::Continue(())
    })?;
  }
  let complete = reason.is_none();
  if complete {
    processed_through = Some(end);
  }
  if let Some(error) = callback_error {
    return Err(error);
  }
  if !initialized && complete {
    initial(output, signals, &mut states, activity, start)?;
  }
  let mut summary_rows = 0u64;
  if cli.summary && complete {
    for (signal, state) in signals.iter().zip(&mut states) {
      state.account(end);
      let [zero, one, x, z, other, unavailable] =
        state.residency.map(|duration| duration.to_string());
      if summary_rows >= cli.max_rows {
        continue;
      }
      if output.record(&json!({
        "type": "scalar_summary",
        "handle": u32::from(signal.handle),
        "duration_ticks": (end - start).to_string(),
        "residency_ticks": {
          "0": zero,
          "1": one,
          "x": x,
          "z": z,
          "other": other,
          "unavailable": unavailable,
        },
        "value_transitions": state.transitions.to_string(),
        "callbacks": state.callbacks.to_string(),
      }))? {
        summary_rows += 1;
      }
    }
  }
  let truncated = if cli.summary {
    complete && summary_rows < signals.len() as u64
  } else {
    matched > emitted
  };
  output.finish(json!({
      "type": "summary",
      "complete": complete,
      "reason": reason.map(ScanStop::as_str),
      "output_reason": truncated.then_some("row_budget_exhausted"),
      "status": if complete { "complete" } else { "partial" },
      "processed_through": processed_through.map(|t| t.to_string()),
      "last_callback_time": last_callback_time.map(|t| t.to_string()),
      "unprocessed_input": !complete,
      "aggregate_final": complete,
      "output_truncated": truncated,
      "selected_handles": signals.len(),
      "decoded_callbacks": callbacks.to_string(),
      "matching_callbacks": matched.to_string(),
      "emitted_events": emitted.to_string(),
      "emitted_rows": (emitted + summary_rows).to_string(),
      "omitted_events": complete.then(|| if cli.summary { "0".to_string() } else { (matched - emitted).to_string() }),
      "observed_omitted_events": if cli.summary { "0".to_string() } else { (matched - emitted).to_string() },
      "truncated": truncated,
    }))?;
  Ok(())
}
