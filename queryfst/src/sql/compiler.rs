//! Validation and lowering of SQL AST expressions into bound expressions.

use crate::error::Result;
use crate::sql::ir::{Aggregate, Expr, ScalarFunction};
use crate::sql::temporal::{Temporal, TemporalKind};
use crate::sql::value::Cell;
use sqlparser::ast::{
  self, BinaryOperator as B, Expr as A, FunctionArg, FunctionArgExpr, FunctionArguments,
  UnaryOperator as U,
};
use std::collections::HashMap;

/// Independent restrictions for aggregate calls and ungrouped column references.
#[derive(Clone, Copy)]
pub(super) struct CompileContext {
  allow_aggregates: bool,
  grouped: bool,
}

impl CompileContext {
  /// WHERE, GROUP BY, aggregate inputs, and temporal inputs see sampled values.
  pub const SAMPLE: Self = Self {
    allow_aggregates: false,
    grouped: false,
  };

  /// SELECT and ORDER BY permit aggregate calls; grouped queries restrict columns.
  pub fn projection(grouped: bool) -> Self {
    Self {
      allow_aggregates: true,
      grouped,
    }
  }
}

/// Aggregate names recognized both by grouping detection and expression lowering.
#[derive(Clone, Copy)]
enum AggregateFunction {
  Count,
  Sum,
  Min,
  Max,
}

impl AggregateFunction {
  fn parse(name: &str) -> Option<Self> {
    Some(match name {
      "count" => Self::Count,
      "sum" => Self::Sum,
      "min" => Self::Min,
      "max" => Self::Max,
      _ => return None,
    })
  }
}

/// Resolves sample bindings and interns aggregate and temporal expressions.
pub(super) struct Compiler<'a> {
  schema: &'a HashMap<String, (usize, u32)>,
  groups: Vec<String>,
  max_pending: usize,
  pub aggregates: Vec<Aggregate>,
  aggregate_keys: HashMap<String, usize>,
  /// Dependency order: nested temporal inputs are registered before their users.
  pub temporal: Vec<Temporal>,
  temporal_keys: HashMap<String, usize>,
}

/// Textual GROUP BY identity, ignoring outer parentheses only.
pub(super) fn key(expr: &A) -> String {
  strip(expr).to_string()
}

fn strip(expr: &A) -> &A {
  match expr {
    A::Nested(e) => strip(e),
    _ => expr,
  }
}

impl<'a> Compiler<'a> {
  pub(super) fn new(
    schema: &'a HashMap<String, (usize, u32)>,
    groups: &[A],
    max_pending: usize,
  ) -> Self {
    Self {
      schema,
      groups: groups.iter().map(key).collect(),
      max_pending,
      aggregates: vec![],
      aggregate_keys: HashMap::new(),
      temporal: vec![],
      temporal_keys: HashMap::new(),
    }
  }

  pub(super) fn compile(&mut self, expr: &A, ctx: CompileContext) -> Result<Expr> {
    if ctx.grouped
      && let Some(index) = self.groups.iter().position(|group| *group == key(expr))
    {
      return Ok(Expr::Group(index));
    }
    Ok(match expr {
      A::Nested(e) => self.compile(e, ctx)?,
      A::Identifier(name) => self.column(name, ctx)?,
      A::CompoundIdentifier(parts)
        if parts.len() == 2 && parts[0].value.eq_ignore_ascii_case("samples") =>
      {
        self.compile(&A::Identifier(parts[1].clone()), ctx)?
      }
      A::Value(value) => Expr::Const(literal(value)?),
      A::UnaryOp { op, expr } => self.unary(op, expr, ctx)?,
      A::BinaryOp { left, op, right } => self.binary(left, op, right, ctx)?,
      A::IsNull(e) => Expr::NullTest(Box::new(self.compile(e, ctx)?), false),
      A::IsNotNull(e) => Expr::NullTest(Box::new(self.compile(e, ctx)?), true),
      A::IsDistinctFrom(a, b) | A::IsNotDistinctFrom(a, b) => Expr::Distinct(
        Box::new(self.compile(a, ctx)?),
        Box::new(self.compile(b, ctx)?),
        matches!(expr, A::IsNotDistinctFrom(_, _)),
      ),
      A::IsTrue(e) | A::IsFalse(e) | A::IsNotTrue(e) | A::IsNotFalse(e) => Expr::TruthTest(
        Box::new(self.compile(e, ctx)?),
        matches!(expr, A::IsTrue(_) | A::IsNotTrue(_)),
        matches!(expr, A::IsNotTrue(_) | A::IsNotFalse(_)),
      ),
      A::Between {
        expr,
        negated,
        low,
        high,
      } => self.between(expr, low, high, *negated, ctx)?,
      A::InList {
        expr,
        list,
        negated,
      } => self.in_list(expr, list, *negated, ctx)?,
      A::Case {
        operand,
        conditions,
        results,
        else_result,
      } => self.case(
        operand.as_deref(),
        conditions,
        results,
        else_result.as_deref(),
        ctx,
      )?,
      A::Function(function) => self.function(function, ctx)?,
      _ => return Err(format!("unsupported SQL expression {expr}").into()),
    })
  }

  fn column(&self, name: &ast::Ident, ctx: CompileContext) -> Result<Expr> {
    if ctx.grouped {
      return Err(format!("column {name} must appear in GROUP BY or an aggregate").into());
    }
    Ok(Expr::Column(
      self
        .schema
        .get(&name.value)
        .ok_or_else(|| format!("unknown column {name}"))?
        .0,
    ))
  }

  fn unary(&mut self, op: &U, expr: &A, ctx: CompileContext) -> Result<Expr> {
    if !matches!(op, U::Not | U::Plus | U::Minus | U::PGBitwiseNot) {
      return Err(format!("unsupported unary operator {op}").into());
    }
    Ok(Expr::Unary(*op, Box::new(self.compile(expr, ctx)?)))
  }

  fn binary(&mut self, left: &A, op: &B, right: &A, ctx: CompileContext) -> Result<Expr> {
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
    Ok(Expr::Binary(
      op.clone(),
      Box::new(self.compile(left, ctx)?),
      Box::new(self.compile(right, ctx)?),
    ))
  }

  fn between(
    &mut self,
    expr: &A,
    low: &A,
    high: &A,
    negated: bool,
    ctx: CompileContext,
  ) -> Result<Expr> {
    let value = self.compile(expr, ctx)?;
    let test = Expr::Binary(
      B::And,
      Box::new(Expr::Binary(
        B::GtEq,
        Box::new(value.clone()),
        Box::new(self.compile(low, ctx)?),
      )),
      Box::new(Expr::Binary(
        B::LtEq,
        Box::new(value),
        Box::new(self.compile(high, ctx)?),
      )),
    );
    Ok(negate(test, negated))
  }

  fn in_list(&mut self, expr: &A, list: &[A], negated: bool, ctx: CompileContext) -> Result<Expr> {
    let value = self.compile(expr, ctx)?;
    let mut test = Expr::Const(Cell::Bool(false));
    for item in list {
      test = Expr::Binary(
        B::Or,
        Box::new(test),
        Box::new(Expr::Binary(
          B::Eq,
          Box::new(value.clone()),
          Box::new(self.compile(item, ctx)?),
        )),
      );
    }
    Ok(negate(test, negated))
  }

  fn case(
    &mut self,
    operand: Option<&A>,
    conditions: &[A],
    results: &[A],
    otherwise: Option<&A>,
    ctx: CompileContext,
  ) -> Result<Expr> {
    Ok(Expr::Case(
      operand
        .map(|e| self.compile(e, ctx).map(Box::new))
        .transpose()?,
      conditions
        .iter()
        .zip(results)
        .map(|(a, b)| Ok((self.compile(a, ctx)?, self.compile(b, ctx)?)))
        .collect::<Result<_>>()?,
      Box::new(
        otherwise
          .map(|e| self.compile(e, ctx))
          .transpose()?
          .unwrap_or(Expr::Const(Cell::Null)),
      ),
    ))
  }

  fn function(&mut self, function: &ast::Function, ctx: CompileContext) -> Result<Expr> {
    let args = function_arguments(function)?;
    let name = function.name.to_string().to_ascii_lowercase();
    if let Some(kind) = AggregateFunction::parse(&name) {
      return self.aggregate(function, &name, kind, &args, ctx);
    }
    let args = args
      .into_iter()
      .collect::<Option<Vec<_>>>()
      .ok_or("wildcard requires COUNT(*)")?;
    if let Some(kind) = TemporalKind::parse(&name, self.max_pending) {
      return self.temporal(function, &name, kind, &args, ctx);
    }
    if name == "raw" {
      return self.raw(&args, ctx);
    }
    let kind =
      ScalarFunction::parse(&name).ok_or_else(|| format!("unsupported function {name}"))?;
    if !kind.accepts_arity(args.len()) {
      return Err(format!("invalid number of arguments for {name}").into());
    }
    Ok(Expr::Function(kind, self.arguments(&args, ctx)?))
  }

  fn arguments(&mut self, args: &[&A], ctx: CompileContext) -> Result<Vec<Expr>> {
    args.iter().map(|arg| self.compile(arg, ctx)).collect()
  }

  fn aggregate(
    &mut self,
    function: &ast::Function,
    name: &str,
    kind: AggregateFunction,
    args: &[Option<&A>],
    ctx: CompileContext,
  ) -> Result<Expr> {
    if !ctx.allow_aggregates {
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
      .map(|arg| self.compile(arg, CompileContext::SAMPLE))
      .transpose()?;
    let spec = match kind {
      AggregateFunction::Count => Aggregate::Count(argument),
      AggregateFunction::Sum => sum(argument.ok_or("SUM(*) is invalid")?),
      AggregateFunction::Min => Aggregate::Min(argument.ok_or("MIN(*) is invalid")?),
      AggregateFunction::Max => Aggregate::Max(argument.ok_or("MAX(*) is invalid")?),
    };
    let index = self.aggregates.len();
    self.aggregates.push(spec);
    self.aggregate_keys.insert(key, index);
    Ok(Expr::Aggregate(index))
  }

  fn temporal(
    &mut self,
    function: &ast::Function,
    name: &str,
    kind: TemporalKind,
    args: &[&A],
    ctx: CompileContext,
  ) -> Result<Expr> {
    if ctx.grouped {
      return Err("temporal expressions must be grouped or used inside an aggregate".into());
    }
    let arity = kind.expected_arity(args.len());
    if args.len() != arity {
      return Err(format!("{name} requires {arity} arguments").into());
    }
    // Preserve textual call identity when sharing history across clauses.
    let key = function.to_string();
    if let Some(index) = self.temporal_keys.get(&key) {
      return Ok(Expr::Temporal(*index));
    }
    // Lower inputs first so nested temporal dependencies precede this node.
    let args = self.arguments(args, CompileContext::SAMPLE)?;
    if matches!(kind, TemporalKind::Timeouts(_))
      && !matches!(args[2], Expr::Const(Cell::Integer(v)) if v >= 0 && v <= i128::from(u64::MAX))
    {
      return Err("timeouts deadline must be a nonnegative u64 integer literal".into());
    }
    let index = self.temporal.len();
    self.temporal.push(Temporal::new(kind, args));
    self.temporal_keys.insert(key, index);
    Ok(Expr::Temporal(index))
  }

  fn raw(&mut self, args: &[&A], ctx: CompileContext) -> Result<Expr> {
    if args.len() != 1 {
      return Err("raw requires one argument".into());
    }
    let width = match strip(args[0]) {
      A::Identifier(id) => self.schema.get(&id.value).map(|v| v.1),
      A::CompoundIdentifier(ids)
        if ids.len() == 2 && ids[0].value.eq_ignore_ascii_case("samples") =>
      {
        self.schema.get(&ids[1].value).map(|v| v.1)
      }
      _ => None,
    };
    Ok(Expr::Raw(Box::new(self.compile(args[0], ctx)?), width))
  }
}

fn literal(value: &ast::Value) -> Result<Cell> {
  Ok(match value {
    ast::Value::Number(v, _) => Cell::Integer(
      v.parse()
        .map_err(|_| "only signed 128-bit integer SQL literals are supported")?,
    ),
    ast::Value::Boolean(v) => Cell::Bool(*v),
    ast::Value::Null => Cell::Null,
    ast::Value::SingleQuotedString(v) | ast::Value::DoubleQuotedString(v) => Cell::Text(v.clone()),
    ast::Value::HexStringLiteral(v) => Cell::Integer(
      i128::from_str_radix(v, 16).map_err(|_| "hex literal outside signed 128-bit range")?,
    ),
    _ => return Err(format!("unsupported literal {value}").into()),
  })
}

fn negate(expr: Expr, negated: bool) -> Expr {
  if negated {
    Expr::Unary(U::Not, Box::new(expr))
  } else {
    expr
  }
}

/// Validates modifiers and preserves the COUNT(*) wildcard until classification.
fn function_arguments(function: &ast::Function) -> Result<Vec<Option<&A>>> {
  if function.filter.is_some()
    || function.over.is_some()
    || function.null_treatment.is_some()
    || !function.within_group.is_empty()
    || !matches!(function.parameters, FunctionArguments::None)
  {
    return Err("FILTER/OVER/function modifiers are unsupported; temporal functions operate on the sampled stream".into());
  }
  let FunctionArguments::List(args) = &function.args else {
    return Err("expected function arguments".into());
  };
  if args.duplicate_treatment.is_some() || !args.clauses.is_empty() {
    return Err("DISTINCT/function argument clauses are unsupported".into());
  }
  args
    .args
    .iter()
    .map(|arg| match arg {
      FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Ok(Some(expr)),
      FunctionArg::Unnamed(FunctionArgExpr::Wildcard) => Ok(None),
      _ => Err("named/qualified function arguments are unsupported".into()),
    })
    .collect()
}

/// Recognizes the bit-sum fast path without changing aggregate registration.
fn sum(value: Expr) -> Aggregate {
  if let Expr::Binary(B::BitwiseAnd, left, right) = &value
    && matches!(**right, Expr::Const(Cell::Integer(1)))
    && let Expr::Binary(B::PGBitwiseShiftRight, column, shift) = &**left
    && let (Expr::Column(index), Expr::Const(Cell::Integer(bit))) = (&**column, &**shift)
    && (0..128).contains(bit)
  {
    Aggregate::SumBit(*index, *bit as u32)
  } else {
    Aggregate::Sum(value)
  }
}

pub(super) fn has_aggregate(expr: &A) -> bool {
  // Used only to choose grouping context; the compiler then checks every AST node.
  match expr {
    A::Function(f)
      if AggregateFunction::parse(&f.name.to_string().to_ascii_lowercase()).is_some() =>
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
