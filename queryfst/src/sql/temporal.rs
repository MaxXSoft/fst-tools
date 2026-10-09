//! Stateful functions advanced on the sampled stream.

use crate::error::Result;
use crate::sql::deadline::Deadline;
use crate::sql::ir::Expr;
use crate::sql::value::Cell;

/// Compiled temporal inputs and the history retained between sampled rows.
pub(super) struct Temporal {
  pub name: String,
  pub args: Vec<Expr>,
  pub previous: Cell,
  pub run: u64,
  pub deadline: Option<Deadline>,
}

impl Temporal {
  /// Advances once per sampled row, before WHERE, in dependency order.
  pub(super) fn advance(&mut self, args: Vec<Cell>) -> Result<Cell> {
    Ok(match self.name.as_str() {
      "timeouts" => self.deadline.as_mut().unwrap().step(&args)?,
      "lag" => {
        let old = self.previous.clone();
        self.previous = args[0].clone();
        old
      }
      "changed" => {
        let changed = self.previous.clone().normalized()? != args[0].clone().normalized()?;
        self.previous = args[0].clone();
        Cell::Bool(changed)
      }
      "hold" => {
        match args[1].truth()? {
          Some(true) => self.previous = args[0].clone(),
          Some(false) => (),
          None => self.previous = Cell::Null,
        }
        self.previous.clone()
      }
      "run_length" | "runs" => {
        match args[0].truth()? {
          Some(true) => self.run = self.run.checked_add(1).ok_or("run length overflow")?,
          Some(false) => self.run = 0,
          None => {
            self.run = 0;
            return Ok(Cell::Null);
          }
        }
        if self.name == "runs" {
          Cell::Bool(self.run == 1)
        } else {
          Cell::Integer(i128::from(self.run))
        }
      }
      _ => return Err(format!("unknown temporal function {}", self.name).into()),
    })
  }
}
