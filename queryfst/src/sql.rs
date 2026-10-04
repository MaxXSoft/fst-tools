//! Finite SQL queries over explicitly sampled waveform state.
mod context;
mod deadline;
mod plan;

use fstapi::Reader;
use std::collections::{BTreeMap, HashMap};
use std::ops::ControlFlow;
use std::time::Instant;

/// A lossless scalar value used by SQL expressions and output rows.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cell {
  Null,
  Bool(bool),
  Integer(i128),
  Text(String),
  /// Four-state or wide logic whose numeric conversion is unavailable.
  Bits(String),
}

impl Cell {
  /// Serializes integers as decimal strings, retaining four-state/wide bits.
  pub fn to_json(&self) -> serde_json::Value {
    match self {
      Self::Null => serde_json::Value::Null,
      Self::Bool(v) => (*v).into(),
      Self::Integer(v) => v.to_string().into(),
      Self::Text(v) => v.clone().into(),
      Self::Bits(v) => serde_json::json!({"bits": v}),
    }
  }

  /// Converts known logic to integers while retaining unknown/wide values.
  fn logic(bytes: &[u8]) -> Result<Self, String> {
    if bytes.iter().all(|c| matches!(c, b'0' | b'1')) && bytes.len() <= 127 {
      Ok(Self::Integer(
        bytes.iter().fold(0, |v, c| (v << 1) | i128::from(c - b'0')),
      ))
    } else if bytes.iter().all(|c| {
      matches!(
        c,
        b'0'
          | b'1'
          | b'x'
          | b'X'
          | b'z'
          | b'Z'
          | b'h'
          | b'H'
          | b'l'
          | b'L'
          | b'u'
          | b'U'
          | b'w'
          | b'W'
          | b'-'
      )
    }) {
      Ok(Self::Bits(
        std::str::from_utf8(bytes)
          .map_err(|e| e.to_string())?
          .into(),
      ))
    } else {
      Err("SQL sampling currently accepts fixed-width logic signals only".into())
    }
  }

  /// Numeric operators propagate unknown values; oversized known values error.
  fn integer(&self) -> Result<Option<i128>, String> {
    match self {
      Self::Null => Ok(None),
      Self::Bool(v) => Ok(Some(i128::from(*v))),
      Self::Integer(v) => Ok(Some(*v)),
      Self::Bits(bits) if bits.bytes().all(|b| matches!(b, b'0' | b'1')) => {
        i128::from_str_radix(bits, 2).map(Some).map_err(|_| {
          "numeric operation exceeds signed 128-bit range; use raw() for wide bits".into()
        })
      }
      Self::Bits(_) => Ok(None),
      Self::Text(_) => Err("numeric operation on text".into()),
    }
  }

  /// Canonicalizes SQL-equivalent known numeric keys without losing unknown bits.
  fn normalized(self) -> Result<Self, String> {
    match &self {
      Self::Bool(value) => Ok(Self::Integer(i128::from(*value))),
      Self::Bits(bits) if bits.bytes().all(|b| matches!(b, b'0' | b'1')) => {
        Ok(Self::Integer(self.integer()?.unwrap()))
      }
      _ => Ok(self),
    }
  }

  /// SQL predicates use three-valued truth, with integer zero treated as false.
  fn truth(&self) -> Result<Option<bool>, String> {
    Ok(self.integer()?.map(|v| v != 0))
  }
}

/// Named output expression.
#[derive(Clone, Debug)]
pub struct Column {
  pub name: String,
}

/// Sampling domain; phase is an absolute raw tick residue modulo period.
#[derive(Clone, Debug)]
pub enum Sampling {
  Period { period: u64, phase: u64 },
}

/// Chooses matching sample windows independently of SQL output row limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
  First,
  Last,
  All,
}

/// Execution parameters, independent of the CLI output format.
#[derive(Clone, Debug)]
pub struct Options {
  pub sql: String,
  pub bindings: BTreeMap<String, String>,
  pub start: u64,
  pub end: u64,
  pub sampling: Sampling,
  pub max_callbacks: Option<u64>,
  pub max_samples: Option<u64>,
  pub max_groups: Option<usize>,
  pub max_buffer_rows: Option<usize>,
  pub max_duration_ms: Option<u64>,
  pub context_before: u64,
  pub context_after: u64,
  pub matches: MatchMode,
}

/// Work/output status; partial aggregation describes only the observed prefix.
#[derive(Clone, Debug, Default)]
pub struct Report {
  pub columns: Vec<Column>,
  pub scan_complete: bool,
  pub pending_requests: u64,
  pub unresolved_due_to_unknown: u64,
  pub unmatched_responses: u64,
  pub context_after_pending: u64,
  pub context_before_clipped: bool,
  pub decoded_callbacks: u64,
  pub sampled_rows: u64,
  pub matched_rows: u64,
  pub emitted_rows: u64,
  pub complete: bool,
  pub output_truncated: bool,
  pub stop_reason: Option<String>,
  pub processed_through: Option<u64>,
}

/// Invokes a bounded SQL plan on complete timestamp snapshots.
///
/// `emit` returning false stops output and requests cooperative traversal stop.
/// Unknown logic propagates SQL NULL through numeric predicates, while raw
/// projections retain bits. Recording interruptions inside the requested range
/// are rejected; pre-range interruptions invalidate earlier observations.
pub fn execute(
  reader: &mut Reader,
  options: &Options,
  mut emit: impl FnMut(&[Column], &[Cell]) -> Result<bool, String>,
) -> Result<Report, String> {
  if options.start > options.end
    || options.start < reader.start_time()
    || options.end > reader.end_time()
  {
    return Err("SQL range must be contained in the waveform and satisfy start <= end".into());
  }
  let Sampling::Period { period, phase } = options.sampling;
  if period == 0 || phase >= period {
    return Err("sampling requires period > 0 and phase < period".into());
  }
  let activity = reader.dump_activity();
  let mut active = true;
  for &(time, next) in &activity {
    if time <= options.start {
      active = next;
    }
    if !next && options.start <= time && time <= options.end {
      return Err("SQL sampling range contains a waveform recording interruption".into());
    }
  }
  if !active {
    return Err("SQL sampling starts while waveform recording is disabled".into());
  }
  let names: Vec<_> = options.bindings.keys().cloned().collect();
  if names
    .iter()
    .any(|n| matches!(n.as_str(), "tick" | "sample_index"))
  {
    return Err("binding names tick and sample_index are reserved".into());
  }
  let mut paths: HashMap<&str, Vec<usize>> = HashMap::new();
  for (i, name) in names.iter().enumerate() {
    paths
      .entry(options.bindings[name].as_str())
      .or_default()
      .push(i);
  }
  let mut mapping = HashMap::new();
  let mut widths = vec![0; names.len()];
  let mut found = vec![false; names.len()];
  let mut handles = Vec::new();
  for entry in reader.vars() {
    let (path, var) = entry.map_err(|e| e.to_string())?;
    if let Some(indices) = paths.get(path.as_str()) {
      if var.length() == 0
        || matches!(
          var.ty(),
          fstapi::var_type::GEN_STRING
            | fstapi::var_type::VCD_REAL
            | fstapi::var_type::VCD_REAL_PARAMETER
            | fstapi::var_type::VCD_REALTIME
            | fstapi::var_type::SV_SHORTREAL
            | fstapi::var_type::VCD_PORT
        )
      {
        return Err(format!("binding {path} is not fixed-width logic"));
      }
      for &i in indices {
        widths[i] = var.length();
        found[i] = true;
      }
      mapping
        .entry(var.handle())
        .or_insert_with(Vec::new)
        .extend(indices.iter().copied());
      handles.push(var.handle());
    }
  }
  if found.iter().any(|v| !v) {
    return Err(format!(
      "missing bound signals: {}",
      names
        .iter()
        .enumerate()
        .filter(|(i, _)| !found[*i])
        .map(|(_, n)| n.as_str())
        .collect::<Vec<_>>()
        .join(", ")
    ));
  }
  let schema: HashMap<_, _> = names
    .iter()
    .enumerate()
    .map(|(i, name)| (name.clone(), (i, widths[i])))
    .chain([
      ("tick".into(), (names.len(), 64)),
      ("sample_index".into(), (names.len() + 1, 64)),
    ])
    .collect();
  let mut plan = plan::Plan::compile(&options.sql, &schema, options)?;
  reader.clear_mask_all();
  for handle in handles {
    reader.set_mask(handle);
  }
  let prior_off: Vec<_> = activity
    .iter()
    .filter(|(t, enabled)| !enabled && *t < options.start)
    .map(|(t, _)| *t)
    .collect();
  reader.set_time_range_limit(
    if prior_off.is_empty() {
      options.start
    } else {
      reader.start_time()
    },
    options.end,
  );
  let mut values = vec![Cell::Null; names.len() + 2];
  let mut observed = vec![None; names.len()];
  let residue = options.start % period;
  let offset = if phase >= residue {
    phase - residue
  } else {
    period - (residue - phase)
  };
  let mut next_sample = options
    .start
    .checked_add(offset)
    .filter(|t| *t <= options.end);
  let mut report = Report {
    columns: plan.columns.clone(),
    ..Report::default()
  };
  if plan.empty_limit() {
    report.complete = true;
    report.stop_reason = Some("sql_limit".into());
    return Ok(report);
  }
  if options.max_callbacks == Some(0)
    || options.max_duration_ms == Some(0)
    || options.max_samples == Some(0)
    || plan.zero_group_budget()
  {
    report.stop_reason = Some(
      if options.max_callbacks == Some(0) {
        "callback_budget"
      } else if options.max_duration_ms == Some(0) {
        "duration_budget"
      } else if options.max_samples == Some(0) {
        "sample_budget"
      } else {
        "group_budget"
      }
      .into(),
    );
    plan.finish(&mut report, &mut emit)?;
    return Ok(report);
  }
  let mut callback_error = None;
  let timer = Instant::now();
  let mut initialized = false;
  let mut emit_samples = |until: u64,
                          inclusive: bool,
                          values: &mut [Cell],
                          observed: &[Option<u64>],
                          report: &mut Report|
   -> Result<bool, String> {
    if !initialized {
      for (i, time) in observed.iter().enumerate() {
        if time.is_some_and(|time| {
          prior_off.iter().any(|off| *off >= time)
            || !activity
              .iter()
              .take_while(|(at, _)| *at <= time)
              .last()
              .map(|(_, active)| *active)
              .unwrap_or(true)
        }) {
          values[i] = Cell::Null;
        }
      }
      initialized = true;
    }
    while let Some(tick) = next_sample.filter(|t| *t < until || inclusive && *t == until) {
      if options
        .max_samples
        .is_some_and(|limit| report.sampled_rows >= limit)
      {
        report.stop_reason = Some("sample_budget".into());
        return Ok(false);
      }
      if options
        .max_duration_ms
        .is_some_and(|limit| timer.elapsed().as_millis() >= u128::from(limit))
      {
        report.stop_reason = Some("duration_budget".into());
        return Ok(false);
      }
      values[names.len()] = Cell::Integer(i128::from(tick));
      values[names.len() + 1] = Cell::Integer(i128::from(report.sampled_rows));
      report.sampled_rows += 1;
      report.processed_through = Some(tick);
      next_sample = tick.checked_add(period).filter(|t| *t <= options.end);
      if !plan.sample(values, report, &mut emit)? {
        return Ok(false);
      }
    }
    Ok(true)
  };
  if mapping.is_empty() {
    if emit_samples(options.end, true, &mut values, &observed, &mut report)? {
      report.complete = true;
      report.scan_complete = true;
    }
    plan.finish(&mut report, &mut emit)?;
    return Ok(report);
  }
  let mut last_time = None;
  let traversed = reader
    .for_each_block_controlled(|time, handle, bytes, _| {
      if callback_error.is_some() {
        return ControlFlow::Break(());
      }
      let result = (|| -> Result<bool, String> {
        if last_time.is_some_and(|last| time < last) {
          return Err("nonmonotonic callback timestamps".into());
        }
        if last_time != Some(time) {
          if time >= options.start
            && !emit_samples(time, false, &mut values, &observed, &mut report)?
          {
            return Ok(false);
          }
          last_time = Some(time);
        }
        // An enclosing block can deliver callbacks beyond the requested end.
        // Every requested sample is now based on a complete preceding group.
        if time > options.end {
          return Ok(false);
        }
        report.decoded_callbacks += 1;
        if options
          .max_duration_ms
          .is_some_and(|limit| timer.elapsed().as_millis() >= u128::from(limit))
        {
          report.stop_reason = Some("duration_budget".into());
          return Ok(false);
        }
        let value = Cell::logic(bytes)?;
        for &index in &mapping[&handle] {
          values[index] = value.clone();
          observed[index] = Some(time);
        }
        if options
          .max_callbacks
          .is_some_and(|limit| report.decoded_callbacks >= limit)
        {
          report.stop_reason = Some("callback_budget".into());
          return Ok(false);
        }
        Ok(true)
      })();
      match result {
        Ok(true) => ControlFlow::Continue(()),
        Ok(false) => ControlFlow::Break(()),
        Err(error) => {
          callback_error = Some(error);
          ControlFlow::Break(())
        }
      }
    })
    .map_err(|e| e.to_string())?;
  if let Some(error) = callback_error {
    return Err(error);
  }
  if report.stop_reason.is_none()
    && (traversed || last_time.is_some_and(|t| t > options.end))
    && emit_samples(options.end, true, &mut values, &observed, &mut report)?
  {
    report.complete = true;
    report.scan_complete = true;
  }
  plan.finish(&mut report, &mut emit)?;
  Ok(report)
}
