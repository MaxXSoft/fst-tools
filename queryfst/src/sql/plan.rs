//! Typed finite SQL planning, temporal expressions, and streaming reductions.
use super::StopReason;
use super::context::Context as MatchContext;
use super::deadline::Deadline;
use super::{Cell, Column, MatchMode, Options, Report};
use crate::error::{Error, Result};
use sqlparser::ast::{
  self as ast, BinaryOperator as B, Expr as A, FunctionArg, FunctionArgExpr, FunctionArguments,
  GroupByExpr, SelectItem, SetExpr, Statement, TableFactor, UnaryOperator as U,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug)]
enum Expr {
  Const(Cell),
  Column(usize),
  Group(usize),
  Aggregate(usize),
  Temporal(usize),
  Unary(U, Box<Self>),
  Binary(B, Box<Self>, Box<Self>),
  TruthTest(Box<Self>, bool, bool),
  NullTest(Box<Self>, bool),
  Distinct(Box<Self>, Box<Self>, bool),
  Case(Option<Box<Self>>, Vec<(Self, Self)>, Box<Self>),
  Function(String, Vec<Self>),
  Raw(Box<Self>, Option<u32>),
}

struct Context<'a> {
  cells: &'a [Cell],
  temporal: &'a [Cell],
  groups: &'a [Cell],
  aggregates: &'a [Cell],
}

impl Expr {
  fn eval(&self, ctx: &Context<'_>) -> Result<Cell> {
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
    _ => return Err(format!("unsupported or invalid binary operator {op}").into()),
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

#[derive(Clone)]
enum Aggregate {
  Count(Option<Expr>),
  Sum(Expr),
  Min(Expr),
  Max(Expr),
  SumBit(usize, u32),
}
impl Aggregate {
  fn update(&self, state: &mut Cell, ctx: &Context<'_>) -> Result<()> {
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
  fn initial(&self) -> Cell {
    if matches!(self, Self::Count(_)) {
      Cell::Integer(0)
    } else {
      Cell::Null
    }
  }
}

struct Temporal {
  name: String,
  args: Vec<Expr>,
  previous: Cell,
  run: u64,
  deadline: Option<Deadline>,
}
impl Temporal {
  fn advance(&mut self, args: Vec<Cell>) -> Result<Cell> {
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

struct Compiler<'a> {
  schema: &'a HashMap<String, (usize, u32)>,
  groups: Vec<String>,
  max_pending: usize,
  aggregates: Vec<Aggregate>,
  aggregate_keys: HashMap<String, usize>,
  temporal: Vec<Temporal>,
  temporal_keys: HashMap<String, usize>,
}

fn key(expr: &A) -> String {
  match expr {
    A::Nested(e) => key(e),
    _ => expr.to_string(),
  }
}
fn strip(expr: &A) -> &A {
  match expr {
    A::Nested(e) => strip(e),
    _ => expr,
  }
}

impl Compiler<'_> {
  fn compile(&mut self, expr: &A, aggregate: bool, grouped: bool) -> Result<Expr> {
    if grouped && let Some(index) = self.groups.iter().position(|group| *group == key(expr)) {
      return Ok(Expr::Group(index));
    }
    Ok(match expr {
      A::Nested(e) => self.compile(e, aggregate, grouped)?,
      A::Identifier(name) => {
        if grouped {
          return Err(format!("column {name} must appear in GROUP BY or an aggregate").into());
        }
        Expr::Column(
          self
            .schema
            .get(&name.value)
            .ok_or_else(|| format!("unknown column {name}"))?
            .0,
        )
      }
      A::CompoundIdentifier(parts)
        if parts.len() == 2 && parts[0].value.eq_ignore_ascii_case("samples") =>
      {
        self.compile(&A::Identifier(parts[1].clone()), aggregate, grouped)?
      }
      A::Value(value) => Expr::Const(match value {
        ast::Value::Number(v, _) => Cell::Integer(
          v.parse()
            .map_err(|_| "only signed 128-bit integer SQL literals are supported")?,
        ),
        ast::Value::Boolean(v) => Cell::Bool(*v),
        ast::Value::Null => Cell::Null,
        ast::Value::SingleQuotedString(v) | ast::Value::DoubleQuotedString(v) => {
          Cell::Text(v.clone())
        }
        ast::Value::HexStringLiteral(v) => Cell::Integer(
          i128::from_str_radix(v, 16).map_err(|_| "hex literal outside signed 128-bit range")?,
        ),
        _ => return Err(format!("unsupported literal {value}").into()),
      }),
      A::UnaryOp { op, expr } => {
        if !matches!(op, U::Not | U::Plus | U::Minus | U::PGBitwiseNot) {
          return Err(format!("unsupported unary operator {op}").into());
        }
        Expr::Unary(*op, Box::new(self.compile(expr, aggregate, grouped)?))
      }
      A::BinaryOp { left, op, right } => {
        if !matches!(
          op,
          B::Plus
            | B::Minus
            | B::Multiply
            | B::Divide
            | B::DuckIntegerDivide
            | B::MyIntegerDivide
            | B::Modulo
            | B::Eq
            | B::NotEq
            | B::Lt
            | B::LtEq
            | B::Gt
            | B::GtEq
            | B::And
            | B::Or
            | B::Xor
            | B::BitwiseAnd
            | B::BitwiseOr
            | B::BitwiseXor
            | B::PGBitwiseXor
            | B::PGBitwiseShiftLeft
            | B::PGBitwiseShiftRight
            | B::StringConcat
        ) {
          return Err(format!("unsupported binary operator {op}").into());
        }
        Expr::Binary(
          op.clone(),
          Box::new(self.compile(left, aggregate, grouped)?),
          Box::new(self.compile(right, aggregate, grouped)?),
        )
      }
      A::IsNull(e) => Expr::NullTest(Box::new(self.compile(e, aggregate, grouped)?), false),
      A::IsNotNull(e) => Expr::NullTest(Box::new(self.compile(e, aggregate, grouped)?), true),
      A::IsDistinctFrom(a, b) | A::IsNotDistinctFrom(a, b) => Expr::Distinct(
        Box::new(self.compile(a, aggregate, grouped)?),
        Box::new(self.compile(b, aggregate, grouped)?),
        matches!(expr, A::IsNotDistinctFrom(_, _)),
      ),
      A::IsTrue(e) | A::IsFalse(e) | A::IsNotTrue(e) | A::IsNotFalse(e) => {
        let expected = matches!(expr, A::IsTrue(_) | A::IsNotTrue(_));
        Expr::TruthTest(
          Box::new(self.compile(e, aggregate, grouped)?),
          expected,
          matches!(expr, A::IsNotTrue(_) | A::IsNotFalse(_)),
        )
      }
      A::Between {
        expr,
        negated,
        low,
        high,
      } => {
        let value = self.compile(expr, aggregate, grouped)?;
        let test = Expr::Binary(
          B::And,
          Box::new(Expr::Binary(
            B::GtEq,
            Box::new(value.clone()),
            Box::new(self.compile(low, aggregate, grouped)?),
          )),
          Box::new(Expr::Binary(
            B::LtEq,
            Box::new(value),
            Box::new(self.compile(high, aggregate, grouped)?),
          )),
        );
        if *negated {
          Expr::Unary(U::Not, Box::new(test))
        } else {
          test
        }
      }
      A::InList {
        expr,
        list,
        negated,
      } => {
        let value = self.compile(expr, aggregate, grouped)?;
        let mut test = Expr::Const(Cell::Bool(false));
        for item in list {
          test = Expr::Binary(
            B::Or,
            Box::new(test),
            Box::new(Expr::Binary(
              B::Eq,
              Box::new(value.clone()),
              Box::new(self.compile(item, aggregate, grouped)?),
            )),
          );
        }
        if *negated {
          Expr::Unary(U::Not, Box::new(test))
        } else {
          test
        }
      }
      A::Case {
        operand,
        conditions,
        results,
        else_result,
      } => Expr::Case(
        operand
          .as_ref()
          .map(|e| self.compile(e, aggregate, grouped).map(Box::new))
          .transpose()?,
        conditions
          .iter()
          .zip(results)
          .map(|(a, b)| {
            Ok((
              self.compile(a, aggregate, grouped)?,
              self.compile(b, aggregate, grouped)?,
            ))
          })
          .collect::<Result<_>>()?,
        Box::new(
          else_result
            .as_ref()
            .map(|e| self.compile(e, aggregate, grouped))
            .transpose()?
            .unwrap_or(Expr::Const(Cell::Null)),
        ),
      ),
      A::Function(function) => self.function(function, aggregate, grouped)?,
      _ => return Err(format!("unsupported SQL expression {expr}").into()),
    })
  }

  fn function(&mut self, function: &ast::Function, aggregate: bool, grouped: bool) -> Result<Expr> {
    if function.filter.is_some()
      || function.over.is_some()
      || function.null_treatment.is_some()
      || !function.within_group.is_empty()
      || !matches!(function.parameters, FunctionArguments::None)
    {
      return Err("FILTER/OVER/function modifiers are unsupported; temporal functions operate on the sampled stream".into());
    }
    let name = function.name.to_string().to_ascii_lowercase();
    let FunctionArguments::List(args) = &function.args else {
      return Err("expected function arguments".into());
    };
    if args.duplicate_treatment.is_some() || !args.clauses.is_empty() {
      return Err("DISTINCT/function argument clauses are unsupported".into());
    }
    let args = args
      .args
      .iter()
      .map(|arg| match arg {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Ok(Some(expr)),
        FunctionArg::Unnamed(FunctionArgExpr::Wildcard) => Ok(None),
        _ => Err("named/qualified function arguments are unsupported".into()),
      })
      .collect::<Result<Vec<_>>>()?;
    let aggregate_function = matches!(name.as_str(), "count" | "sum" | "min" | "max");
    if aggregate_function {
      if !aggregate {
        return Err(
          "aggregate functions cannot be nested or used in WHERE/GROUP BY/temporal inputs".into(),
        );
      }
      if args.len() != 1 {
        return Err(format!("{name} requires one argument").into());
      }
      let key = function.to_string();
      if let Some(index) = self.aggregate_keys.get(&key) {
        return Ok(Expr::Aggregate(*index));
      }
      let argument = args[0]
        .map(|arg| self.compile(arg, false, false))
        .transpose()?;
      let spec = match name.as_str() {
        "count" => Aggregate::Count(argument),
        "sum" => {
          let value = argument.ok_or("SUM(*) is invalid")?;
          if let Expr::Binary(B::BitwiseAnd, left, right) = &value
            && matches!(**right, Expr::Const(Cell::Integer(1)))
            && let Expr::Binary(B::PGBitwiseShiftRight, column, shift) = &**left
            && let (Expr::Column(index), Expr::Const(Cell::Integer(bit))) = (&**column, &**shift)
            && (0..128).contains(bit)
          {
            let spec = Aggregate::SumBit(*index, *bit as u32);
            let index = self.aggregates.len();
            self.aggregates.push(spec);
            self.aggregate_keys.insert(key, index);
            return Ok(Expr::Aggregate(index));
          }
          Aggregate::Sum(value)
        }
        "min" => Aggregate::Min(argument.ok_or("MIN(*) is invalid")?),
        _ => Aggregate::Max(argument.ok_or("MAX(*) is invalid")?),
      };
      let index = self.aggregates.len();
      self.aggregates.push(spec);
      self.aggregate_keys.insert(key, index);
      return Ok(Expr::Aggregate(index));
    }
    if args.iter().any(Option::is_none) {
      return Err("wildcard requires COUNT(*)".into());
    }
    if matches!(
      name.as_str(),
      "lag" | "changed" | "hold" | "run_length" | "runs" | "timeouts"
    ) {
      if grouped {
        return Err("temporal expressions must be grouped or used inside an aggregate".into());
      }
      let arity = if name == "timeouts" {
        args.len().clamp(3, 4)
      } else if name == "hold" {
        2
      } else {
        1
      };
      if args.len() != arity {
        return Err(format!("{name} requires {arity} arguments").into());
      }
      let key = function.to_string();
      if let Some(index) = self.temporal_keys.get(&key) {
        return Ok(Expr::Temporal(*index));
      }
      let args = args
        .iter()
        .map(|arg| self.compile(arg.unwrap(), false, false))
        .collect::<Result<Vec<_>>>()?;
      if name == "timeouts"
        && !matches!(args[2], Expr::Const(Cell::Integer(v)) if v >= 0 && v <= i128::from(u64::MAX))
      {
        return Err("timeouts deadline must be a nonnegative u64 integer literal".into());
      }
      let deadline = (name == "timeouts").then(|| Deadline::new(self.max_pending));
      let index = self.temporal.len();
      self.temporal.push(Temporal {
        name,
        args,
        previous: Cell::Null,
        run: 0,
        deadline,
      });
      self.temporal_keys.insert(key, index);
      return Ok(Expr::Temporal(index));
    }
    if name == "raw" {
      if args.len() != 1 {
        return Err("raw requires one argument".into());
      }
      let width = match strip(args[0].unwrap()) {
        A::Identifier(id) => self.schema.get(&id.value).map(|v| v.1),
        A::CompoundIdentifier(ids)
          if ids.len() == 2 && ids[0].value.eq_ignore_ascii_case("samples") =>
        {
          self.schema.get(&ids[1].value).map(|v| v.1)
        }
        _ => None,
      };
      return Ok(Expr::Raw(
        Box::new(self.compile(args[0].unwrap(), aggregate, grouped)?),
        width,
      ));
    }
    let valid_arity = match name.as_str() {
      "coalesce" => !args.is_empty(),
      "known" | "is_known" | "abs" | "hex" => args.len() == 1,
      "bit" => args.len() == 2,
      _ => return Err(format!("unsupported function {name}").into()),
    };
    if !valid_arity {
      return Err(format!("invalid number of arguments for {name}").into());
    }
    Ok(Expr::Function(
      name,
      args
        .iter()
        .map(|arg| self.compile(arg.unwrap(), aggregate, grouped))
        .collect::<Result<_>>()?,
    ))
  }
}

struct Group {
  keys: Vec<Cell>,
  aggregates: Vec<Cell>,
}
struct Row {
  values: Vec<Cell>,
  order: Vec<Cell>,
}
struct Order {
  expression: OrderExpression,
  ascending: bool,
  nulls_first: bool,
}
enum OrderExpression {
  Projection(usize),
  Expression(Expr),
}

/// Compiled plan containing only supported operations, never arbitrary SQL.
pub(super) struct Plan {
  pub columns: Vec<Column>,
  projection: Vec<Expr>,
  selection: Option<Expr>,
  group_by: Vec<Expr>,
  aggregate: bool,
  aggregates: Vec<Aggregate>,
  group_indices: HashMap<Vec<Cell>, usize>,
  groups: Vec<Group>,
  key_scratch: Vec<Cell>,
  temporal: Vec<Temporal>,
  temporal_values: Vec<Cell>,
  order: Vec<Order>,
  natural_order: bool,
  context: Option<MatchContext>,
  tail_limit: Option<usize>,
  tail: VecDeque<Row>,
  rows: Vec<Row>,
  limit: Option<u64>,
  output_rows: u64,
  max_groups: Option<usize>,
  max_buffer_rows: Option<usize>,
}

fn has_aggregate(expr: &A) -> bool {
  // Used only to choose grouping context; the compiler then checks every AST node.
  match expr {
    A::Function(f)
      if matches!(
        f.name.to_string().to_ascii_lowercase().as_str(),
        "count" | "sum" | "min" | "max"
      ) =>
    {
      true
    }
    A::Nested(e)
    | A::UnaryOp { expr: e, .. }
    | A::IsNull(e)
    | A::IsNotNull(e)
    | A::IsTrue(e)
    | A::IsNotTrue(e)
    | A::IsFalse(e)
    | A::IsNotFalse(e) => has_aggregate(e),
    A::IsDistinctFrom(a, b) | A::IsNotDistinctFrom(a, b) => has_aggregate(a) || has_aggregate(b),
    A::Between {
      expr, low, high, ..
    } => has_aggregate(expr) || has_aggregate(low) || has_aggregate(high),
    A::InList { expr, list, .. } => has_aggregate(expr) || list.iter().any(has_aggregate),
    A::BinaryOp { left, right, .. } => has_aggregate(left) || has_aggregate(right),
    A::Case {
      operand,
      conditions,
      results,
      else_result,
    } => {
      operand.as_ref().is_some_and(|e| has_aggregate(e))
        || conditions.iter().chain(results).any(has_aggregate)
        || else_result.as_ref().is_some_and(|e| has_aggregate(e))
    }
    A::Function(f) => {
      if let FunctionArguments::List(args) = &f.args {
        args.args.iter().any(
          |arg| matches!(arg, FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) if has_aggregate(e)),
        )
      } else {
        false
      }
    }
    _ => false,
  }
}

impl Plan {
  pub(super) fn compile(
    sql: &str,
    schema: &HashMap<String, (usize, u32)>,
    options: &Options,
  ) -> Result<Self> {
    let mut statements = Parser::parse_sql(&GenericDialect {}, sql)?;
    if statements.len() != 1 {
      return Err("exactly one SELECT statement is required".into());
    }
    let Statement::Query(query) = statements.remove(0) else {
      return Err("only SELECT queries are supported".into());
    };
    if query.with.is_some()
      || query.offset.is_some()
      || query.fetch.is_some()
      || !query.limit_by.is_empty()
      || !query.locks.is_empty()
      || query.for_clause.is_some()
      || query.settings.is_some()
      || query.format_clause.is_some()
    {
      return Err("WITH/OFFSET/FETCH and query modifiers are unsupported".into());
    }
    let SetExpr::Select(select) = &*query.body else {
      return Err("set operations/subqueries are unsupported".into());
    };
    if select.distinct.is_some()
      || select.top.is_some()
      || select.into.is_some()
      || !select.lateral_views.is_empty()
      || select.prewhere.is_some()
      || !select.cluster_by.is_empty()
      || !select.distribute_by.is_empty()
      || !select.sort_by.is_empty()
      || select.having.is_some()
      || !select.named_window.is_empty()
      || select.qualify.is_some()
      || select.value_table_mode.is_some()
      || select.connect_by.is_some()
    {
      return Err("unsupported SELECT modifier".into());
    }
    if select.from.len() != 1 || !select.from[0].joins.is_empty() {
      return Err("FROM samples is required; joins are unsupported".into());
    }
    match &select.from[0].relation {
      TableFactor::Table {
        name,
        alias,
        args,
        with_hints,
        version,
        partitions,
        with_ordinality,
        json_path,
      } if name.to_string().eq_ignore_ascii_case("samples")
        && alias.is_none()
        && args.is_none()
        && with_hints.is_empty()
        && version.is_none()
        && !with_ordinality
        && json_path.is_none()
        && partitions.is_empty() => {}
      _ => return Err("only the samples virtual table without modifiers is supported".into()),
    }
    let GroupByExpr::Expressions(group_ast, modifiers) = &select.group_by else {
      return Err("GROUP BY ALL is unsupported".into());
    };
    if !modifiers.is_empty() {
      return Err("GROUP BY modifiers are unsupported".into());
    }
    let mut projection_ast = Vec::new();
    let mut columns = Vec::new();
    for item in &select.projection {
      match item {
        SelectItem::UnnamedExpr(expr) => {
          columns.push(Column {
            name: expr.to_string(),
          });
          projection_ast.push(expr.clone());
        }
        SelectItem::ExprWithAlias { expr, alias } => {
          columns.push(Column {
            name: alias.value.clone(),
          });
          projection_ast.push(expr.clone());
        }
        _ => return Err("SELECT * is unsupported; select named bindings explicitly".into()),
      }
    }
    if columns.is_empty() {
      return Err("empty SELECT projection".into());
    }
    let aggregate = !group_ast.is_empty()
      || projection_ast.iter().any(has_aggregate)
      || query
        .order_by
        .as_ref()
        .is_some_and(|o| o.exprs.iter().any(|e| has_aggregate(&e.expr)));
    let mut compiler = Compiler {
      schema,
      groups: group_ast.iter().map(key).collect(),
      max_pending: options.max_buffer_rows.unwrap_or(100000),
      aggregates: vec![],
      aggregate_keys: HashMap::new(),
      temporal: vec![],
      temporal_keys: HashMap::new(),
    };
    let selection = select
      .selection
      .as_ref()
      .map(|expr| compiler.compile(expr, false, false))
      .transpose()?;
    let group_by = group_ast
      .iter()
      .map(|expr| compiler.compile(expr, false, false))
      .collect::<Result<Vec<_>>>()?;
    let projection = projection_ast
      .iter()
      .map(|expr| compiler.compile(expr, true, aggregate))
      .collect::<Result<Vec<_>>>()?;
    let mut order = Vec::new();
    if let Some(order_by) = &query.order_by {
      if order_by.interpolate.is_some() {
        return Err("ORDER BY INTERPOLATE is unsupported".into());
      }
      for item in &order_by.exprs {
        if item.with_fill.is_some() {
          return Err("ORDER BY WITH FILL is unsupported".into());
        }
        let projected = match &item.expr {
          A::Identifier(name) => columns.iter().position(|c| c.name == name.value),
          A::Value(ast::Value::Number(value, _)) => Some(
            value
              .parse::<usize>()
              .ok()
              .and_then(|v| v.checked_sub(1))
              .filter(|v| *v < columns.len())
              .ok_or("ORDER BY ordinal is outside the SELECT projection")?,
          ),
          _ => projection_ast
            .iter()
            .position(|expr| key(expr) == key(&item.expr)),
        };
        let expression = match projected {
          Some(i) => OrderExpression::Projection(i),
          None => OrderExpression::Expression(compiler.compile(&item.expr, true, aggregate)?),
        };
        order.push(Order {
          expression,
          ascending: item.asc.unwrap_or(true),
          nulls_first: item.nulls_first.unwrap_or(false),
        });
      }
    }
    let limit = query
      .limit
      .as_ref()
      .map(|expr| -> Result<u64> {
        match expr {
          A::Value(ast::Value::Number(v, _)) => v
            .parse::<u64>()
            .map_err(|_| "LIMIT requires a nonnegative u64 integer".into()),
          _ => Err("LIMIT requires a nonnegative integer literal".into()),
        }
      })
      .transpose()?;
    let natural_order = !aggregate
      && (order.is_empty()
        || order.len() == 1
          && order[0].ascending
          && match &order[0].expression {
            OrderExpression::Projection(i) => {
              matches!(projection[*i], Expr::Column(index) if index == schema["tick"].0 || index == schema["sample_index"].0)
            }
            OrderExpression::Expression(Expr::Column(index)) => {
              *index == schema["tick"].0 || *index == schema["sample_index"].0
            }
            _ => false,
          });
    let tail_limit = if !aggregate
      && order.len() == 1
      && !order[0].ascending
      && match &order[0].expression {
        OrderExpression::Projection(i) => {
          matches!(projection[*i], Expr::Column(index) if index == schema["tick"].0 || index == schema["sample_index"].0)
        }
        OrderExpression::Expression(Expr::Column(index)) => {
          *index == schema["tick"].0 || *index == schema["sample_index"].0
        }
        _ => false,
      } {
      limit
        .map(|limit| usize::try_from(limit).map_err(|_| "LIMIT exceeds address space".to_string()))
        .transpose()?
    } else {
      None
    };
    if tail_limit.is_some_and(|limit| options.max_buffer_rows.is_some_and(|cap| limit > cap)) {
      return Err("descending time LIMIT exceeds max-buffer-rows".into());
    }
    let context = if options.context_before > 0
      || options.context_after > 0
      || options.matches != MatchMode::All
    {
      if limit.is_some() {
        return Err(
          "SQL LIMIT cannot be combined with match context; use --matches first/last/all".into(),
        );
      }
      if aggregate || !natural_order {
        return Err("context windows require ungrouped samples in natural order".into());
      }
      if columns.iter().any(|c| c.name == "__match") {
        return Err("__match is reserved for context output".into());
      }
      columns.push(Column {
        name: "__match".into(),
      });
      Some(MatchContext::new(
        options.context_before,
        options.context_after,
        options.max_buffer_rows.unwrap_or(100000),
        options.matches,
      )?)
    } else {
      None
    };
    let temporal_values = vec![Cell::Null; compiler.temporal.len()];
    let mut plan = Self {
      columns,
      projection,
      selection,
      group_by,
      aggregate,
      aggregates: compiler.aggregates,
      group_indices: HashMap::new(),
      groups: vec![],
      key_scratch: vec![],
      temporal: compiler.temporal,
      temporal_values,
      order,
      natural_order,
      context,
      tail_limit,
      tail: VecDeque::new(),
      rows: vec![],
      limit,
      output_rows: 0,
      max_groups: options.max_groups,
      max_buffer_rows: options.max_buffer_rows,
    };
    if aggregate && group_ast.is_empty() && options.max_groups != Some(0) {
      plan.groups.push(Group {
        keys: vec![],
        aggregates: plan.aggregates.iter().map(Aggregate::initial).collect(),
      });
      plan.group_indices.insert(vec![], 0);
    }
    Ok(plan)
  }

  pub(super) fn empty_limit(&self) -> bool {
    self.limit == Some(0)
  }

  pub(super) fn zero_group_budget(&self) -> bool {
    self.aggregate && self.group_by.is_empty() && self.max_groups == Some(0)
  }

  /// Advances temporal expressions once, before evaluating WHERE.
  pub(super) fn sample(
    &mut self,
    cells: &[Cell],
    report: &mut Report,
    emit: &mut impl FnMut(&[Column], &[Cell]) -> Result<bool>,
  ) -> Result<bool> {
    for i in 0..self.temporal.len() {
      let ctx = Context {
        cells,
        temporal: &self.temporal_values,
        groups: &[],
        aggregates: &[],
      };
      let args = self.temporal[i]
        .args
        .iter()
        .map(|expr| expr.eval(&ctx))
        .collect::<Result<Vec<_>>>()?;
      self.temporal_values[i] = match self.temporal[i].advance(args) {
        Ok(value) => value,
        Err(Error::PendingRequestBudget) => {
          report.stop_reason = Some(StopReason::PendingRequestBudget);
          return Ok(false);
        }
        Err(error) => return Err(error),
      };
    }
    let ctx = Context {
      cells,
      temporal: &self.temporal_values,
      groups: &[],
      aggregates: &[],
    };
    let matched = self
      .selection
      .as_ref()
      .map(|e| e.eval(&ctx)?.truth())
      .transpose()?
      .unwrap_or(Some(true))
      == Some(true);
    if matched {
      report.matched_rows += 1;
    }
    if let Some(context) = &mut self.context {
      let values = self
        .projection
        .iter()
        .map(|expr| expr.eval(&ctx))
        .collect::<Result<Vec<_>>>()?;
      for row in context.push(report.sampled_rows - 1, values, matched) {
        if self.limit.is_some_and(|limit| self.output_rows >= limit) {
          continue;
        }
        if !emit(&self.columns, &row)? {
          report.output_truncated = true;
          report.stop_reason = Some(StopReason::OutputLimit);
          return Ok(false);
        }
        self.output_rows += 1;
        report.emitted_rows += 1;
      }
      if context.done() {
        report.complete = true;
        report.stop_reason = Some(StopReason::FirstMatch);
        return Ok(false);
      }
      return Ok(true);
    }
    if !matched {
      return Ok(true);
    }
    if self.aggregate {
      self.key_scratch.clear();
      for expr in &self.group_by {
        self.key_scratch.push(expr.eval(&ctx)?.normalized()?);
      }
      let index = match self.group_indices.get(self.key_scratch.as_slice()) {
        Some(index) => *index,
        None => {
          if self
            .max_groups
            .is_some_and(|limit| self.groups.len() >= limit)
          {
            report.stop_reason = Some(StopReason::GroupBudget);
            return Ok(false);
          }
          let index = self.groups.len();
          self.group_indices.insert(self.key_scratch.clone(), index);
          self.groups.push(Group {
            keys: self.key_scratch.clone(),
            aggregates: self.aggregates.iter().map(Aggregate::initial).collect(),
          });
          index
        }
      };
      for (spec, state) in self
        .aggregates
        .iter()
        .zip(&mut self.groups[index].aggregates)
      {
        spec.update(state, &ctx)?;
      }
    } else {
      let values = self
        .projection
        .iter()
        .map(|expr| expr.eval(&ctx))
        .collect::<Result<Vec<_>>>()?;
      if self.natural_order {
        if self.limit.is_none_or(|limit| self.output_rows < limit) {
          if !emit(&self.columns, &values)? {
            report.output_truncated = true;
            report.stop_reason = Some(StopReason::OutputLimit);
            return Ok(false);
          }
          self.output_rows += 1;
          report.emitted_rows += 1;
          if self.limit == Some(self.output_rows) {
            report.complete = true;
            report.stop_reason = Some(StopReason::SqlLimit);
            return Ok(false);
          }
        }
      } else {
        if self.tail_limit.is_none()
          && self
            .max_buffer_rows
            .is_some_and(|limit| self.rows.len() >= limit)
        {
          report.stop_reason = Some(StopReason::BufferBudget);
          return Ok(false);
        }
        let order = self
          .order
          .iter()
          .map(|order| match &order.expression {
            OrderExpression::Projection(i) => values[*i].clone().normalized(),
            OrderExpression::Expression(expr) => expr.eval(&ctx)?.normalized(),
          })
          .collect::<Result<_>>()?;
        let row = Row { values, order };
        if let Some(limit) = self.tail_limit {
          if limit > 0 {
            if self.tail.len() == limit {
              self.tail.pop_front();
            }
            self.tail.push_back(row);
          }
        } else {
          self.rows.push(row);
        }
      }
    }
    Ok(true)
  }

  /// Finalizes groups and ordering, also exposing provisional prefix aggregates.
  pub(super) fn finish(
    &mut self,
    report: &mut Report,
    emit: &mut impl FnMut(&[Column], &[Cell]) -> Result<bool>,
  ) -> Result<()> {
    if self.aggregate && self.group_by.is_empty() && self.max_groups == Some(0) {
      report.complete = false;
      report.stop_reason = Some(StopReason::GroupBudget);
    }
    for temporal in &self.temporal {
      if let Some(deadline) = &temporal.deadline {
        report.pending_requests += deadline.pending();
        report.unresolved_due_to_unknown += deadline.invalidated;
        report.unmatched_responses += deadline.unmatched_responses;
      }
    }
    if let Some(context) = &mut self.context {
      report.context_after_pending = context.pending_after();
      report.context_before_clipped = context.before_clipped;
      for row in context.finish() {
        if !emit(&self.columns, &row)? {
          report.output_truncated = true;
          if report.stop_reason.is_none() {
            report.stop_reason = Some(StopReason::OutputLimit);
          }
          break;
        }
        self.output_rows += 1;
        report.emitted_rows += 1;
      }
    }
    self.rows.extend(self.tail.drain(..));
    if self.aggregate {
      for group in &self.groups {
        let ctx = Context {
          cells: &[],
          temporal: &self.temporal_values,
          groups: &group.keys,
          aggregates: &group.aggregates,
        };
        let values = self
          .projection
          .iter()
          .map(|expr| expr.eval(&ctx))
          .collect::<Result<Vec<_>>>()?;
        let order = self
          .order
          .iter()
          .map(|order| match &order.expression {
            OrderExpression::Projection(i) => values[*i].clone().normalized(),
            OrderExpression::Expression(expr) => expr.eval(&ctx)?.normalized(),
          })
          .collect::<Result<_>>()?;
        if self.order.is_empty() {
          if self.limit.is_none_or(|limit| self.output_rows < limit) {
            if !emit(&self.columns, &values)? {
              report.output_truncated = true;
              if report.stop_reason.is_none() {
                report.stop_reason = Some(StopReason::OutputLimit);
              }
              break;
            }
            self.output_rows += 1;
            report.emitted_rows += 1;
          }
        } else {
          if self
            .max_buffer_rows
            .is_some_and(|limit| self.rows.len() >= limit)
          {
            report.stop_reason = Some(StopReason::BufferBudget);
            report.complete = false;
            report.output_truncated = true;
            break;
          }
          self.rows.push(Row { values, order });
        }
      }
    }
    if !self.order.is_empty() {
      self.rows.sort_by(|a, b| {
        for (index, order) in self.order.iter().enumerate() {
          let (left, right) = (&a.order[index], &b.order[index]);
          let result = match (matches!(left, Cell::Null), matches!(right, Cell::Null)) {
            (true, true) => Ordering::Equal,
            (true, false) => {
              if order.nulls_first {
                Ordering::Less
              } else {
                Ordering::Greater
              }
            }
            (false, true) => {
              if order.nulls_first {
                Ordering::Greater
              } else {
                Ordering::Less
              }
            }
            (false, false) => {
              if order.ascending {
                left.cmp(right)
              } else {
                right.cmp(left)
              }
            }
          };
          if result != Ordering::Equal {
            return result;
          }
        }
        Ordering::Equal
      });
    }
    for row in &self.rows {
      if self.limit.is_some_and(|limit| self.output_rows >= limit) {
        break;
      }
      if !emit(&self.columns, &row.values)? {
        report.output_truncated = true;
        if report.stop_reason.is_none() {
          report.stop_reason = Some(StopReason::OutputLimit);
        }
        break;
      }
      self.output_rows += 1;
      report.emitted_rows += 1;
    }
    Ok(())
  }
}
