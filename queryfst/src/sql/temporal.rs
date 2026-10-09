//! Stateful functions advanced on the sampled stream.

use crate::error::Result;
use crate::sql::deadline::Deadline;
use crate::sql::functions::TemporalFunction;
use crate::sql::ir::{EvalContext, Expr};
use crate::sql::value::Cell;

/// Compiled temporal inputs and the history retained between sampled rows.
/// Nodes are registered after their dependencies and advanced before WHERE.
pub(super) struct Temporal {
  kind: TemporalKind,
  args: Vec<Expr>,
}

impl Temporal {
  /// Inputs have compiler-validated arity and contain no aggregate references.
  pub(super) fn new(function: TemporalFunction, args: Vec<Expr>, max_pending: usize) -> Self {
    Self {
      kind: TemporalKind::new(function, max_pending),
      args,
    }
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

/// Each temporal operation owns only the history required by that operation.
enum TemporalKind {
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
  /// Constructs history only when the compiler registers a distinct temporal call.
  fn new(function: TemporalFunction, max_pending: usize) -> Self {
    match function {
      TemporalFunction::Timeouts => Self::Timeouts(Deadline::new(max_pending)),
      TemporalFunction::Lag => Self::Lag(Cell::Null),
      TemporalFunction::Changed => Self::Changed(Cell::Null),
      TemporalFunction::Hold => Self::Hold(Cell::Null),
      TemporalFunction::RunLength => Self::RunLength(0),
      TemporalFunction::Runs => Self::Runs(0),
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
