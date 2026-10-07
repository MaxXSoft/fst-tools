//! Timestamp-complete sampling and bounded traversal.
use super::{
  Cell, Column, Options, PeriodicSampling, Report, StopReason,
  bindings::{self, ResolvedSignals},
  plan::Plan,
};
use crate::error::{Error, Result};
use fstapi::{Handle, Reader};
use std::ops::ControlFlow;
use std::time::Instant;

struct Recording {
  activity: Vec<(u64, bool)>,
  prior_off: Vec<u64>,
  scan_start: u64,
}

fn validate(reader: &Reader, options: &Options) -> Result<Recording> {
  if options.start > options.end
    || options.start < reader.start_time()
    || options.end > reader.end_time()
  {
    return Err(Error::Arguments(
      "SQL range must be contained in the waveform and satisfy start <= end".into(),
    ));
  }
  let PeriodicSampling { period, phase } = options.sampling;
  if period == 0 || phase >= period {
    return Err(Error::Arguments(
      "sampling requires period > 0 and phase < period".into(),
    ));
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
  let prior_off: Vec<_> = activity
    .iter()
    .filter(|(t, enabled)| !enabled && *t < options.start)
    .map(|(t, _)| *t)
    .collect();
  let scan_start = if prior_off.is_empty() {
    options.start
  } else {
    reader.start_time()
  };
  Ok(Recording {
    activity,
    prior_off,
    scan_start,
  })
}

struct Executor<'a> {
  options: &'a Options,
  recording: Recording,
  signals: ResolvedSignals,
  plan: Plan,
  values: Vec<Cell>,
  observed: Vec<Option<u64>>,
  next_sample: Option<u64>,
  initialized: bool,
  last_time: Option<u64>,
  timer: Instant,
  report: Report,
}

impl<'a> Executor<'a> {
  fn new(options: &'a Options, recording: Recording, signals: ResolvedSignals, plan: Plan) -> Self {
    let PeriodicSampling { period, phase } = options.sampling;
    let residue = options.start % period;
    let offset = if phase >= residue {
      phase - residue
    } else {
      period - (residue - phase)
    };
    let next_sample = options
      .start
      .checked_add(offset)
      .filter(|t| *t <= options.end);
    let values = vec![Cell::Null; signals.sample_index + 1];
    let observed = vec![None; signals.tick_index];
    let report = Report {
      columns: plan.columns.clone(),
      ..Report::default()
    };
    Self {
      options,
      recording,
      signals,
      plan,
      values,
      observed,
      next_sample,
      initialized: false,
      last_time: None,
      timer: Instant::now(),
      report,
    }
  }

  fn initial_stop(&mut self) -> bool {
    if self.plan.empty_limit() {
      self.report.complete = true;
      self.report.stop_reason = Some(StopReason::SqlLimit);
      return true;
    }
    self.report.stop_reason = if self.options.max_callbacks == Some(0) {
      Some(StopReason::CallbackBudget)
    } else if self.options.max_duration_ms == Some(0) {
      Some(StopReason::DurationBudget)
    } else if self.options.max_samples == Some(0) {
      Some(StopReason::SampleBudget)
    } else if self.plan.zero_group_budget() {
      Some(StopReason::GroupBudget)
    } else {
      None
    };
    self.report.stop_reason.is_some()
  }

  /// Emit only snapshots whose entire timestamp has already been consumed.
  fn emit_samples(
    &mut self,
    until: u64,
    inclusive: bool,
    emit: &mut impl FnMut(&[Column], &[Cell]) -> Result<bool>,
  ) -> Result<bool> {
    if !self.initialized {
      for (i, time) in self.observed.iter().enumerate() {
        if time.is_some_and(|time| {
          self.recording.prior_off.iter().any(|off| *off >= time)
            || !self
              .recording
              .activity
              .iter()
              .take_while(|(at, _)| *at <= time)
              .last()
              .map(|(_, active)| *active)
              .unwrap_or(true)
        }) {
          self.values[i] = Cell::Null;
        }
      }
      self.initialized = true;
    }
    while let Some(tick) = self
      .next_sample
      .filter(|t| *t < until || inclusive && *t == until)
    {
      if self
        .options
        .max_samples
        .is_some_and(|limit| self.report.sampled_rows >= limit)
      {
        self.report.stop_reason = Some(StopReason::SampleBudget);
        return Ok(false);
      }
      if self
        .options
        .max_duration_ms
        .is_some_and(|limit| self.timer.elapsed().as_millis() >= u128::from(limit))
      {
        self.report.stop_reason = Some(StopReason::DurationBudget);
        return Ok(false);
      }
      self.values[self.signals.tick_index] = Cell::Integer(i128::from(tick));
      self.values[self.signals.sample_index] = Cell::Integer(i128::from(self.report.sampled_rows));
      self.report.sampled_rows += 1;
      self.report.processed_through = Some(tick);
      self.next_sample = tick
        .checked_add(self.options.sampling.period)
        .filter(|t| *t <= self.options.end);
      if !self.plan.sample(&self.values, &mut self.report, emit)? {
        return Ok(false);
      }
    }
    Ok(true)
  }

  fn callback(
    &mut self,
    time: u64,
    handle: Handle,
    bytes: &[u8],
    emit: &mut impl FnMut(&[Column], &[Cell]) -> Result<bool>,
  ) -> Result<bool> {
    if self.last_time.is_some_and(|last| time < last) {
      return Err("nonmonotonic callback timestamps".into());
    }
    if self.last_time != Some(time) {
      if time >= self.options.start && !self.emit_samples(time, false, emit)? {
        return Ok(false);
      }
      self.last_time = Some(time);
    }
    // An enclosing block can deliver callbacks beyond the requested end.
    // Every requested sample is now based on a complete preceding group.
    if time > self.options.end {
      return Ok(false);
    }
    self.report.decoded_callbacks += 1;
    if self
      .options
      .max_duration_ms
      .is_some_and(|limit| self.timer.elapsed().as_millis() >= u128::from(limit))
    {
      self.report.stop_reason = Some(StopReason::DurationBudget);
      return Ok(false);
    }
    let value = Cell::logic(bytes)?;
    for &index in &self.signals.mapping[&handle] {
      self.values[index] = value.clone();
      self.observed[index] = Some(time);
    }
    if self
      .options
      .max_callbacks
      .is_some_and(|limit| self.report.decoded_callbacks >= limit)
    {
      self.report.stop_reason = Some(StopReason::CallbackBudget);
      return Ok(false);
    }
    Ok(true)
  }

  fn scan(
    &mut self,
    reader: &mut Reader,
    emit: &mut impl FnMut(&[Column], &[Cell]) -> Result<bool>,
  ) -> Result<()> {
    if self.signals.mapping.is_empty() {
      if self.emit_samples(self.options.end, true, emit)? {
        self.report.complete = true;
        self.report.scan_complete = true;
      }
      return Ok(());
    }
    let mut callback_error = None;
    let traversed = reader.for_each_block_controlled(|time, handle, bytes, _| {
      match self.callback(time, handle, bytes, emit) {
        Ok(true) => ControlFlow::Continue(()),
        Ok(false) => ControlFlow::Break(()),
        Err(error) => {
          callback_error = Some(error);
          ControlFlow::Break(())
        }
      }
    })?;
    if let Some(error) = callback_error {
      return Err(error);
    }
    if self.report.stop_reason.is_none()
      && (traversed || self.last_time.is_some_and(|t| t > self.options.end))
      && self.emit_samples(self.options.end, true, emit)?
    {
      self.report.complete = true;
      self.report.scan_complete = true;
    }
    Ok(())
  }
}

pub(super) fn execute(
  reader: &mut Reader,
  options: &Options,
  mut emit: impl FnMut(&[Column], &[Cell]) -> Result<bool>,
) -> Result<Report> {
  let recording = validate(reader, options)?;
  let signals = bindings::resolve(reader, &options.bindings)?;
  let plan = Plan::compile(&options.sql, &signals.schema, options)?;
  reader.clear_mask_all();
  for &handle in signals.mapping.keys() {
    reader.set_mask(handle);
  }
  reader.set_time_range_limit(recording.scan_start, options.end);
  let mut executor = Executor::new(options, recording, signals, plan);
  let stopped = executor.initial_stop();
  if executor.report.complete {
    return Ok(executor.report);
  }
  if !stopped {
    executor.scan(reader, &mut emit)?;
  }
  executor.plan.finish(&mut executor.report, &mut emit)?;
  Ok(executor.report)
}
