//! Finite SQL queries over explicitly sampled waveform state.

mod bindings;
mod context;
mod deadline;
mod executor;
mod plan;

use crate::error::Result;
use fstapi::Reader;
use std::collections::BTreeMap;

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
  fn logic(bytes: &[u8]) -> Result<Self> {
    if bytes.iter().all(|c| matches!(c, b'0' | b'1')) && bytes.len() <= 127 {
      Ok(Self::Integer(
        bytes.iter().fold(0, |v, c| (v << 1) | i128::from(c - b'0')),
      ))
    } else if bytes.iter().all(|c| {
      matches!(
        c.to_ascii_lowercase(),
        b'0' | b'1' | b'x' | b'z' | b'h' | b'l' | b'u' | b'w' | b'-'
      )
    }) {
      Ok(Self::Bits(std::str::from_utf8(bytes)?.into()))
    } else {
      Err("SQL sampling currently accepts fixed-width logic signals only".into())
    }
  }

  /// Numeric operators propagate unknown values; oversized known values error.
  fn integer(&self) -> Result<Option<i128>> {
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
  fn normalized(self) -> Result<Self> {
    match &self {
      Self::Bool(value) => Ok(Self::Integer(i128::from(*value))),
      Self::Bits(bits) if bits.bytes().all(|b| matches!(b, b'0' | b'1')) => {
        Ok(Self::Integer(self.integer()?.unwrap()))
      }
      _ => Ok(self),
    }
  }

  /// SQL predicates use three-valued truth, with integer zero treated as false.
  fn truth(&self) -> Result<Option<bool>> {
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
pub struct PeriodicSampling {
  pub period: u64,
  pub phase: u64,
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
  pub sampling: PeriodicSampling,
  pub max_callbacks: Option<u64>,
  pub max_samples: Option<u64>,
  pub max_groups: Option<usize>,
  pub max_buffer_rows: Option<usize>,
  pub max_duration_ms: Option<u64>,
  pub context_before: u64,
  pub context_after: u64,
  pub matches: MatchMode,
}

/// A cooperative stop may complete a finite query without scanning the whole trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
  SqlLimit,
  FirstMatch,
  CallbackBudget,
  DurationBudget,
  SampleBudget,
  GroupBudget,
  BufferBudget,
  PendingRequestBudget,
  OutputLimit,
}

impl StopReason {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::SqlLimit => "sql_limit",
      Self::FirstMatch => "first_match",
      Self::CallbackBudget => "callback_budget",
      Self::DurationBudget => "duration_budget",
      Self::SampleBudget => "sample_budget",
      Self::GroupBudget => "group_budget",
      Self::BufferBudget => "buffer_budget",
      Self::PendingRequestBudget => "pending_request_budget",
      Self::OutputLimit => "output_limit",
    }
  }
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
  pub stop_reason: Option<StopReason>,
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
  emit: impl FnMut(&[Column], &[Cell]) -> Result<bool>,
) -> Result<Report> {
  executor::execute(reader, options, emit)
}
