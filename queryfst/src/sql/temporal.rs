//! Stateful functions advanced on the sampled stream.

use crate::error::Result;
use crate::sql::deadline::Deadline;
use crate::sql::ir::{EvalContext, Expr};
use crate::sql::value::Cell;

/// Compiled temporal inputs and the history retained between sampled rows.
/// Nodes are registered after their dependencies and advanced before WHERE.
pub(super) struct Temporal {
  kind: TemporalKind,
  args: Vec<Expr>,
}

/// Each temporal operation owns only the history required by that operation.
pub(super) enum TemporalKind {
  /// Bounded request/response tracker, initially empty.
  Timeouts(Deadline),
  /// Previous sample value, initially NULL.
  Lag(Cell),
  /// Previous value compared after normalization, initially NULL.
  Changed(Cell),
  /// Last captured value, initially NULL; an unknown enable invalidates it.
  Hold(Cell),
  /// Consecutive true samples, initially zero; false or unknown resets the count.
  RunLength(u64),
  /// Same count as RunLength, yielding true only at the start of a run.
  Runs(u64),
}

impl TemporalKind {
  /// Resolves a lowercase SQL name once and constructs its initial state.
  pub(super) fn parse(name: &str, max_pending: usize) -> Option<Self> {
    Some(match name {
      "timeouts" => Self::Timeouts(Deadline::new(max_pending)),
      "lag" => Self::Lag(Cell::Null),
      "changed" => Self::Changed(Cell::Null),
      "hold" => Self::Hold(Cell::Null),
      "run_length" => Self::RunLength(0),
      "runs" => Self::Runs(0),
      _ => return None,
    })
  }

  /// Returns the accepted arity nearest to the supplied count for diagnostics.
  pub(super) fn expected_arity(&self, count: usize) -> usize {
    match self {
      Self::Timeouts(_) => count.clamp(3, 4),
      Self::Hold(_) => 2,
      Self::Lag(_) | Self::Changed(_) | Self::RunLength(_) | Self::Runs(_) => 1,
    }
  }

  fn advance(&mut self, args: &[Cell]) -> Result<Cell> {
    Ok(match self {
      Self::Timeouts(deadline) => deadline.step(args)?,
      Self::Lag(previous) => std::mem::replace(previous, args[0].clone()),
      Self::Changed(previous) => {
        let changed = previous.clone().normalized()? != args[0].clone().normalized()?;
        *previous = args[0].clone();
        Cell::Bool(changed)
      }
      Self::Hold(previous) => {
        match args[1].truth()? {
          Some(true) => *previous = args[0].clone(),
          Some(false) => (),
          None => *previous = Cell::Null,
        }
        previous.clone()
      }
      Self::RunLength(run) => advance_run(run, &args[0])?
        .map(|length| Cell::Integer(i128::from(length)))
        .unwrap_or(Cell::Null),
      Self::Runs(run) => advance_run(run, &args[0])?
        .map(|length| Cell::Bool(length == 1))
        .unwrap_or(Cell::Null),
    })
  }
}

impl Temporal {
  /// Inputs have compiler-validated arity and contain no aggregate references.
  pub(super) fn new(kind: TemporalKind, args: Vec<Expr>) -> Self {
    Self { kind, args }
  }

  /// Advances once per sampled row, after dependencies and before WHERE.
  pub(super) fn advance(&mut self, ctx: &EvalContext<'_>) -> Result<Cell> {
    let args = self
      .args
      .iter()
      .map(|expr| expr.eval(ctx))
      .collect::<Result<Vec<_>>>()?;
    self.kind.advance(&args)
  }

  /// Exposes timeout statistics without making the other state variants public.
  pub(super) fn deadline(&self) -> Option<&Deadline> {
    match &self.kind {
      TemporalKind::Timeouts(deadline) => Some(deadline),
      _ => None,
    }
  }
}

/// Unknown input resets history but yields NULL for this sample.
fn advance_run(run: &mut u64, value: &Cell) -> Result<Option<u64>> {
  match value.truth()? {
    Some(true) => *run = run.checked_add(1).ok_or("run length overflow")?,
    Some(false) => *run = 0,
    None => {
      *run = 0;
      return Ok(None);
    }
  }
  Ok(Some(*run))
}
