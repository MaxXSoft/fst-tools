//! Bound SQL expressions, reductions, ordering rules, and their evaluation.

use crate::error::Result;
use crate::sql::value::Cell;
use sqlparser::ast::{BinaryOperator as B, UnaryOperator as U};
use std::cmp::Ordering;

/// Bound expression; indices address the corresponding EvalContext slices.
/// Temporal and aggregate references read values computed by the plan.
#[derive(Clone, Debug)]
pub(super) enum Expr {
  /// Literal value, evaluated without consulting the context.
  Const(Cell),
  /// Index into EvalContext::cells, including tick and sample_index bindings.
  Column(usize),
  /// Index into EvalContext::groups, in GROUP BY order.
  Group(usize),
  /// Index into EvalContext::aggregates, in compiler registration order.
  Aggregate(usize),
  /// Index into EvalContext::temporal, in dependency order.
  Temporal(usize),
  Unary(U, Box<Self>),
  Binary(B, Box<Self>, Box<Self>),
  /// Operand, expected truth value, and whether the IS test is negated.
  TruthTest(Box<Self>, bool, bool),
  /// Operand and whether the IS NULL test is negated.
  NullTest(Box<Self>, bool),
  /// Operands and whether IS DISTINCT FROM is negated.
  Distinct(Box<Self>, Box<Self>, bool),
  /// Optional simple-CASE operand, ordered condition/result pairs, and fallback.
  Case(Option<Box<Self>>, Vec<(Self, Self)>, Box<Self>),
  Function(String, Vec<Self>),
  /// Operand and original signal width when a direct binding can supply it.
  Raw(Box<Self>, Option<u32>),
}

/// Borrowed inputs for one sample or one finalized group.
/// The compiler ensures expressions only address slices available in that phase.
pub(super) struct EvalContext<'a> {
  /// Current sampled values; empty while finalizing groups.
  pub cells: &'a [Cell],
  /// Results of temporal nodes already advanced for this sample.
  pub temporal: &'a [Cell],
  /// Normalized keys for the group being finalized; empty during sampling.
  pub groups: &'a [Cell],
  /// Reduction results for that group; empty during sampling.
  pub aggregates: &'a [Cell],
}

impl Expr {
  pub(super) fn eval(&self, ctx: &EvalContext<'_>) -> Result<Cell> {
    Ok(match self {
      Self::Const(v) => v.clone(),
      Self::Column(i) => ctx.cells[*i].clone(),
      Self::Group(i) => ctx.groups[*i].clone(),
      Self::Aggregate(i) => ctx.aggregates[*i].clone(),
      Self::Temporal(i) => ctx.temporal[*i].clone(),
      Self::Unary(op, expr) => {
        let value = expr.eval(ctx)?;
        match op {
          U::Not => value.truth()?.map(|v| Cell::Bool(!v)).unwrap_or(Cell::Null),
          U::Plus => value.integer()?.map(Cell::Integer).unwrap_or(Cell::Null),
          U::Minus => match value.integer()? {
            Some(v) => Cell::Integer(v.checked_neg().ok_or("integer overflow")?),
            None => Cell::Null,
          },
          U::PGBitwiseNot => value
            .integer()?
            .map(|v| Cell::Integer(!v))
            .unwrap_or(Cell::Null),
          _ => return Err(format!("unsupported unary operator {op}").into()),
        }
      }
      Self::Binary(op, left, right) => binary(op, left.eval(ctx)?, right.eval(ctx)?)?,
      Self::TruthTest(expr, expected, negated) => {
        Cell::Bool((expr.eval(ctx)?.truth()? == Some(*expected)) != *negated)
      }
      Self::NullTest(expr, negated) => {
        Cell::Bool(matches!(expr.eval(ctx)?, Cell::Null) != *negated)
      }
      Self::Distinct(a, b, negated) => {
        Cell::Bool((a.eval(ctx)?.normalized()? != b.eval(ctx)?.normalized()?) != *negated)
      }
      Self::Case(operand, clauses, otherwise) => {
        let operand = operand.as_ref().map(|e| e.eval(ctx)).transpose()?;
        let mut result = None;
        for (condition, value) in clauses {
          let condition = condition.eval(ctx)?;
          let yes = if let Some(operand) = &operand {
            binary(&B::Eq, operand.clone(), condition)?.truth()?
          } else {
            condition.truth()?
          };
          if yes == Some(true) {
            result = Some(value.eval(ctx)?);
            break;
          }
        }
        match result {
          Some(value) => value,
          None => otherwise.eval(ctx)?,
        }
      }
      Self::Raw(expr, width) => match expr.eval(ctx)? {
        Cell::Bits(bits) => Cell::Text(bits),
        Cell::Integer(v) => Cell::Text(if let Some(width) = width {
          format!("{v:0width$b}", width = *width as usize)
        } else {
          format!("{v:b}")
        }),
        value => value,
      },
      Self::Function(name, args) => {
        if name == "coalesce" {
          for argument in args {
            let value = argument.eval(ctx)?;
            if !matches!(value, Cell::Null) {
              return Ok(value);
            }
          }
          return Ok(Cell::Null);
        }
        let args = args
          .iter()
          .map(|e| e.eval(ctx))
          .collect::<Result<Vec<_>>>()?;
        match name.as_str() {
          "coalesce" => args
            .into_iter()
            .find(|v| !matches!(v, Cell::Null))
            .unwrap_or(Cell::Null),
          "known" | "is_known" => Cell::Bool(match &args[0] {
            Cell::Null => false,
            Cell::Bits(v) => v.bytes().all(|b| matches!(b, b'0' | b'1')),
            _ => true,
          }),
          "abs" => match args[0].integer()? {
            Some(v) => Cell::Integer(v.checked_abs().ok_or("integer overflow")?),
            None => Cell::Null,
          },
          "bit" => match args[1].integer()? {
            Some(bit) if (0..128).contains(&bit) => match &args[0] {
              Cell::Bits(bits) => bits
                .as_bytes()
                .iter()
                .rev()
                .nth(bit as usize)
                .and_then(|b| match b {
                  b'0' => Some(0),
                  b'1' => Some(1),
                  _ => None,
                })
                .map(Cell::Integer)
                .unwrap_or(Cell::Null),
              value => value
                .integer()?
                .map(|v| Cell::Integer((v >> bit) & 1))
                .unwrap_or(Cell::Null),
            },
            None => Cell::Null,
            _ => return Err("bit index must be 0..127".into()),
          },
          "hex" => args[0]
            .integer()?
            .map(|v| Cell::Text(format!("{v:x}")))
            .unwrap_or(Cell::Null),
          _ => return Err(format!("unknown function {name}").into()),
        }
      }
    })
  }
}

/// Implements checked integer operations and SQL three-valued predicates.
fn binary(op: &B, left: Cell, right: Cell) -> Result<Cell> {
  if matches!(op, B::And | B::Or | B::Xor) {
    let (a, b) = (left.truth()?, right.truth()?);
    return Ok(match op {
      B::And => {
        if a == Some(false) || b == Some(false) {
          Cell::Bool(false)
        } else if a == Some(true) && b == Some(true) {
          Cell::Bool(true)
        } else {
          Cell::Null
        }
      }
      B::Or => {
        if a == Some(true) || b == Some(true) {
          Cell::Bool(true)
        } else if a == Some(false) && b == Some(false) {
          Cell::Bool(false)
        } else {
          Cell::Null
        }
      }
      _ => a
        .zip(b)
        .map(|(a, b)| Cell::Bool(a ^ b))
        .unwrap_or(Cell::Null),
    });
  }
  if matches!(left, Cell::Null) || matches!(right, Cell::Null) {
    return Ok(Cell::Null);
  }
  if let (Cell::Text(a), Cell::Text(b)) = (&left, &right) {
    if matches!(op, B::StringConcat) {
      return Ok(Cell::Text(format!("{a}{b}")));
    }
    return comparison(op, a.cmp(b)).map(Cell::Bool);
  }
  let (Some(a), Some(b)) = (left.integer()?, right.integer()?) else {
    return Ok(Cell::Null);
  };
  if matches!(op, B::Eq | B::NotEq | B::Gt | B::GtEq | B::Lt | B::LtEq) {
    return comparison(op, a.cmp(&b)).map(Cell::Bool);
  }
  let value = match op {
    B::Plus => a.checked_add(b),
    B::Minus => a.checked_sub(b),
    B::Multiply => a.checked_mul(b),
    B::Divide | B::DuckIntegerDivide | B::MyIntegerDivide => a.checked_div(b),
    B::Modulo => a.checked_rem(b),
    B::BitwiseAnd => Some(a & b),
    B::BitwiseOr => Some(a | b),
    B::BitwiseXor | B::PGBitwiseXor => Some(a ^ b),
    B::PGBitwiseShiftLeft if (0..128).contains(&b) => a.checked_shl(b as u32),
    B::PGBitwiseShiftRight if (0..128).contains(&b) => a.checked_shr(b as u32),
    _ => return Err(format!("unsupported or invalid binary operation {a} {op} {b}").into()),
  }
  .ok_or("integer overflow or division by zero")?;
  Ok(Cell::Integer(value))
}

fn comparison(op: &B, order: Ordering) -> Result<bool> {
  Ok(match op {
    B::Eq => order == Ordering::Equal,
    B::NotEq => order != Ordering::Equal,
    B::Gt => order == Ordering::Greater,
    B::GtEq => order != Ordering::Less,
    B::Lt => order == Ordering::Less,
    B::LtEq => order != Ordering::Greater,
    _ => return Err(format!("invalid comparison {op}").into()),
  })
}

/// Compiled reduction shared by all groups; each group owns a separate Cell state.
#[derive(Clone)]
pub(super) enum Aggregate {
  /// None implements COUNT(*); Some counts non-NULL expression results.
  Count(Option<Expr>),
  Sum(Expr),
  Min(Expr),
  Max(Expr),
  /// Specialized SUM((column >> bit) & 1), with a validated bit in 0..128.
  SumBit(usize, u32),
}

impl Aggregate {
  pub(super) fn update(&self, state: &mut Cell, ctx: &EvalContext<'_>) -> Result<()> {
    match self {
      Self::Count(expr) => {
        if expr
          .as_ref()
          .map(|e| e.eval(ctx))
          .transpose()?
          .is_none_or(|v| !matches!(v, Cell::Null))
        {
          let n = state
            .integer()?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("count overflow")?;
          *state = Cell::Integer(n);
        }
      }
      Self::Sum(expr) => {
        if let Some(value) = expr.eval(ctx)?.integer()? {
          *state = Cell::Integer(
            state
              .integer()?
              .unwrap_or(0)
              .checked_add(value)
              .ok_or("sum overflow")?,
          );
        }
      }
      Self::SumBit(index, bit) => {
        if let Some(value) = ctx.cells[*index].integer()? {
          *state = Cell::Integer(
            state
              .integer()?
              .unwrap_or(0)
              .checked_add((value >> bit) & 1)
              .ok_or("sum overflow")?,
          );
        }
      }
      Self::Min(expr) | Self::Max(expr) => {
        let value = expr.eval(ctx)?;
        let value = match value {
          Cell::Text(_) => value,
          _ => value.integer()?.map(Cell::Integer).unwrap_or(Cell::Null),
        };
        if !matches!(value, Cell::Null)
          && (matches!(state, Cell::Null)
            || if matches!(self, Self::Min(_)) {
              value < *state
            } else {
              value > *state
            })
        {
          *state = value;
        }
      }
    }
    Ok(())
  }

  /// COUNT starts at zero; other reductions remain NULL until a value contributes.
  pub(super) fn initial(&self) -> Cell {
    if matches!(self, Self::Count(_)) {
      Cell::Integer(0)
    } else {
      Cell::Null
    }
  }
}

/// One ORDER BY key; NULL placement is independent of sort direction.
pub(super) struct Order {
  pub expression: OrderExpression,
  pub ascending: bool,
  pub nulls_first: bool,
}

/// Source of a sort key, resolved during compilation.
pub(super) enum OrderExpression {
  /// Zero-based index into the SELECT projection.
  Projection(usize),
  /// Additional expression evaluated in the same context as the projection.
  Expression(Expr),
}
