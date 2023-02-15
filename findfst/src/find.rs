use crate::checker::{DenseChecker, DenseOnceChecker, SparseChecker, SparseOnceChecker};
use crate::checker::{VarChecker, VarInfo};
use crate::matcher::{ExactMatcher, RegexHexMatcher, RegexMatcher, ValueMatcher};
use crate::printer::{FullPrinter, NamePrinter, Printer};
use fstapi::{Handle, Reader, Result};
use regex::{bytes::Regex, Error as RegexError};
use std::fmt;

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

/// Finds the matching value in the given FST waveform.
pub fn find_value(
  reader: &mut Reader,
  value_match: MatchInfo,
  vars: VarInfo,
  all_matches: bool,
  names_only: bool,
) -> Result<()> {
  match value_match {
    MatchInfo::Regex(re, true) => {
      find_value_m(reader, RegexMatcher::new(re), vars, all_matches, names_only)
    }
    MatchInfo::Regex(re, false) => find_value_m(
      reader,
      RegexHexMatcher::new(re),
      vars,
      all_matches,
      names_only,
    ),
    MatchInfo::Exact(e) => {
      find_value_m(reader, ExactMatcher::new(e), vars, all_matches, names_only)
    }
  }
}

/// Stage #2, with value matcher applied, determines variable checker.
fn find_value_m<M>(
  reader: &mut Reader,
  value_matcher: M,
  vars: VarInfo,
  all_matches: bool,
  names_only: bool,
) -> Result<()>
where
  M: ValueMatcher,
{
  match (vars, all_matches) {
    (VarInfo::Map(vars), false) => {
      find_value_mc(reader, value_matcher, SparseChecker::new(vars), names_only)
    }
    (VarInfo::Map(vars), true) => find_value_mc(
      reader,
      value_matcher,
      SparseOnceChecker::new(vars),
      names_only,
    ),
    (VarInfo::Array(vars), false) => {
      find_value_mc(reader, value_matcher, DenseChecker::new(vars), names_only)
    }
    (VarInfo::Array(vars), true) => find_value_mc(
      reader,
      value_matcher,
      DenseOnceChecker::new(vars),
      names_only,
    ),
  }
}

/// Stage #3, with value matcher and variable checker applied, determines printer.
fn find_value_mc<M, T, C>(
  reader: &mut Reader,
  value_matcher: M,
  var_checker: C,
  names_only: bool,
) -> Result<()>
where
  M: ValueMatcher,
  C: VarChecker<T> + Clone,
{
  if names_only {
    find_value_mcp(reader, value_matcher, var_checker, NamePrinter)
  } else {
    find_value_mcp(reader, value_matcher, var_checker, FullPrinter)
  }
}

/// Final stage, all generics are applied, creates callbacks and
/// finds for matching values.
fn find_value_mcp<M, T, C, P>(
  reader: &mut Reader,
  value_matcher: M,
  var_checker: C,
  printer: P,
) -> Result<()>
where
  M: ValueMatcher,
  C: VarChecker<T> + Clone,
  P: Printer,
{
  let mut var_checker = var_checker;
  let mut var_checker2 = var_checker.clone();
  let callback = |time, handle, value: &[u8]| {
    find_value_callback(
      &value_matcher,
      &mut var_checker,
      &printer,
      time,
      handle,
      value,
    )
  };
  let callback2 = |time, handle, value: &[u8]| {
    find_value_callback(
      &value_matcher,
      &mut var_checker2,
      &printer,
      time,
      handle,
      value,
    )
  };
  reader.for_each_block(callback, callback2)
}

/// Callback of FST block iterator.
/// Runs value matcher, variable checker and printer.
fn find_value_callback<M, T, C, P>(
  value_matcher: &M,
  var_checker: &mut C,
  printer: &P,
  time: u64,
  handle: Handle,
  value: &[u8],
) where
  M: ValueMatcher,
  C: VarChecker<T>,
  P: Printer,
{
  // Check if value matches.
  if value_matcher.is_match(value) {
    // Check the current variable and print.
    if let Some(name) = var_checker.check(handle) {
      printer.print(time, name, value);
    }
  }
}
