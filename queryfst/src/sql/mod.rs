//! Finite SQL queries over explicitly sampled waveform state.

#[cfg(test)]
mod bench;
mod bindings;
mod compiler;
mod context;
mod deadline;
mod executor;
mod expr_index;
mod ir;
mod normalize;
mod plan;
mod temporal;
pub(crate) mod value;

use crate::error::Result;
use crate::sql::value::{Cell, Column};
use fstapi::Reader;
use std::collections::BTreeMap;

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
