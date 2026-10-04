use crate::checker::{DenseChecker, DenseOnceChecker, SparseChecker, SparseOnceChecker};
use crate::checker::{VarChecker, VarInfo};
use crate::matcher::{ExactMatcher, RegexHexMatcher, RegexMatcher, ValueMatcher};
use crate::output::Output;
use crate::{Cli, Result};
use fstapi::Reader;
use regex::{Error as RegexError, bytes::Regex};
use std::fmt;
use std::io::Write;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

/// Errors that can occurr when constructing [`MatchInfo`].
pub enum Error {
  Regex(RegexError),
  InvalidHex(String),
  InvalidBin(String),
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Regex(e) => write!(f, "Invalid value regex: {e}"),
      Self::InvalidHex(v) => write!(f, "Invalid hexadecimal value: {v}!"),
      Self::InvalidBin(v) => write!(f, "Invalid binary value: {v}!"),
    }
  }
}

/// Information for matching values.
pub enum MatchInfo {
  Regex(Regex, bool),
  Exact(Box<[u8]>),
}

impl MatchInfo {
  pub fn new(value: String, hex: bool, regex: bool) -> std::result::Result<Self, Error> {
    if regex {
      let re = Regex::new(&value).map_err(Error::Regex)?;
      Ok(Self::Regex(re, hex))
    } else if hex {
      let mut s = Vec::new();
      for c in value.chars() {
        let digit = match c.to_digit(16) {
          Some(d) => d,
          _ => return Err(Error::InvalidHex(value)),
        };
        s.push(if (digit & 8) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 4) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 2) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 1) != 0 { b'1' } else { b'0' });
      }
      Ok(Self::Exact(s.into()))
    } else if value.contains(|c: char| !c.is_digit(2)) {
      Err(Error::InvalidBin(value))
    } else {
      Ok(Self::Exact(value.into_bytes().into()))
    }
  }
}

/// Traversal completion is independent of whether output was truncated.
pub(crate) struct Scan {
  pub complete: bool,
  pub callbacks: u64,
  pub stop_reason: Option<&'static str>,
  pub last_callback_time: Option<u64>,
  pub processed_through: Option<u64>,
}

/// Finds matching callback observations within an exact inclusive window.
pub(crate) fn find_value<W: Write>(
  reader: &mut Reader,
  value_match: MatchInfo,
  vars: VarInfo,
  cli: &Cli,
  start: u64,
  end: u64,
  output: &mut Output<'_, W>,
) -> Result<Scan> {
  match value_match {
    MatchInfo::Regex(re, false) => {
      find_value_m(reader, RegexMatcher::new(re), vars, cli, start, end, output)
    }
    MatchInfo::Regex(re, true) => find_value_m(
      reader,
      RegexHexMatcher::new(re),
      vars,
      cli,
      start,
      end,
      output,
    ),
    MatchInfo::Exact(exact) => find_value_m(
      reader,
      ExactMatcher::new(exact),
      vars,
      cli,
      start,
      end,
      output,
    ),
  }
}

/// Keep the existing sparse/dense and first-per-handle matching semantics.
fn find_value_m<M: ValueMatcher, W: Write>(
  reader: &mut Reader,
  matcher: M,
  vars: VarInfo,
  cli: &Cli,
  start: u64,
  end: u64,
  output: &mut Output<'_, W>,
) -> Result<Scan> {
  match (vars, cli.all_matches) {
    (VarInfo::Map(vars), true) => scan(
      reader,
      matcher,
      SparseChecker::new(vars),
      cli,
      start,
      end,
      output,
    ),
    (VarInfo::Map(vars), false) => scan(
      reader,
      matcher,
      SparseOnceChecker::new(vars),
      cli,
      start,
      end,
      output,
    ),
    (VarInfo::Array(vars), true) => scan(
      reader,
      matcher,
      DenseChecker::new(vars),
      cli,
      start,
      end,
      output,
    ),
    (VarInfo::Array(vars), false) => scan(
      reader,
      matcher,
      DenseOnceChecker::new(vars),
      cli,
      start,
      end,
      output,
    ),
  }
}

/// Work limits stop through libfst's controlled callback rather than hiding work.
fn scan<M, T, C, W>(
  reader: &mut Reader,
  matcher: M,
  mut checker: C,
  cli: &Cli,
  start: u64,
  end: u64,
  output: &mut Output<'_, W>,
) -> Result<Scan>
where
  M: ValueMatcher,
  C: VarChecker<T>,
  W: Write,
{
  let mut scan = Scan {
    complete: true,
    callbacks: 0,
    stop_reason: None,
    last_callback_time: None,
    processed_through: None,
  };
  if checker.num_vars() == 0 {
    scan.processed_through = Some(end);
    return Ok(scan);
  }
  if cli.max_callbacks == Some(0) || cli.max_duration_ms == Some(0) {
    scan.complete = false;
    scan.stop_reason = Some(if cli.max_callbacks == Some(0) {
      "callback_budget_exhausted"
    } else {
      "duration_budget_exhausted"
    });
    return Ok(scan);
  }
  let began = Instant::now();
  let duration = cli.max_duration_ms.map(Duration::from_millis);
  let mut output_error = None;
  let mut passed_end = false;
  let completed = reader.for_each_block_controlled(|time, handle, value, _| {
    scan.callbacks += 1;
    if scan.last_callback_time != Some(time) {
      scan.processed_through = time.checked_sub(1).filter(|&time| time >= start);
    }
    scan.last_callback_time = Some(time);
    // Callback times are ordered. Crossing end proves the requested search
    // complete, even when the enclosing compressed block contains later data.
    if time > end {
      passed_end = true;
      return ControlFlow::Break(());
    }
    if duration.is_some_and(|duration| began.elapsed() >= duration) {
      scan.stop_reason = Some("duration_budget_exhausted");
      return ControlFlow::Break(());
    }
    if start <= time
      && time <= end
      && matcher.is_match(value)
      && let Some(name) = checker.check(handle)
      && let Err(error) = output.matched(time, handle, name, value)
    {
      output_error = Some(error);
      return ControlFlow::Break(());
    }
    if cli
      .max_callbacks
      .is_some_and(|limit| scan.callbacks >= limit)
    {
      scan.stop_reason = Some("callback_budget_exhausted");
      return ControlFlow::Break(());
    }
    ControlFlow::Continue(())
  });
  if let Some(error) = output_error {
    return Err(error);
  }
  scan.complete = completed? || passed_end;
  if scan.complete {
    scan.processed_through = Some(end);
  }
  Ok(scan)
}
