//! Validation and lowering of SQL AST expressions into bound expressions.

use crate::error::Result;
use crate::sql::expr_index::ExprIndex;
use crate::sql::functions::{self, AggregateFunction, BuiltinFunction, TemporalFunction};
use crate::sql::ir::{Aggregate, Expr};
use crate::sql::temporal::Temporal;
use crate::sql::value::Cell;
use sqlparser::ast::{
  self, BinaryOperator as B, Expr as A, FunctionArg, FunctionArgExpr, FunctionArguments,
  UnaryOperator as U,
};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

/// Borrows a canonical call and hashes its AST once for lookup and insertion.
/// Full AST equality still resolves hash collisions.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CallKey<'a> {
  fingerprint: u64,
  function: &'a ast::Function,
}

impl<'a> CallKey<'a> {
  fn new(function: &'a ast::Function, hasher: &impl BuildHasher) -> Self {
    Self {
      function,
      fingerprint: hasher.hash_one(function),
    }
  }
}

impl Hash for CallKey<'_> {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.fingerprint.hash(state);
  }
}

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

/// Resolves sample bindings and interns aggregate and temporal expressions.
pub(super) struct Compiler<'a, 'ast> {
  schema: &'a HashMap<String, (usize, u32)>,
  groups: ExprIndex<'ast>,
  max_pending: usize,
  pub aggregates: Vec<Aggregate>,
  aggregate_keys: HashMap<CallKey<'ast>, usize>,
  /// Dependency order: nested temporal inputs are registered before their users.
  pub temporal: Vec<Temporal>,
  temporal_keys: HashMap<CallKey<'ast>, usize>,
}

fn strip(expr: &A) -> &A {
  match expr {
    A::Nested(e) => strip(e),
    _ => expr,
  }
}

impl<'a, 'ast> Compiler<'a, 'ast> {
  pub(super) fn new(
    schema: &'a HashMap<String, (usize, u32)>,
    groups: impl ExactSizeIterator<Item = &'ast A>,
    max_pending: usize,
  ) -> Self {
    Self {
      schema,
      groups: ExprIndex::new(groups),
      max_pending,
      aggregates: vec![],
      aggregate_keys: HashMap::new(),
      temporal: vec![],
      temporal_keys: HashMap::new(),
    }
  }

  pub(super) fn compile(&mut self, expr: &'ast A, ctx: CompileContext) -> Result<Expr> {
    if ctx.grouped
      && let Some(index) = self.groups.get(expr)
    {
      return Ok(Expr::Group(index));
    }
    Ok(match expr {
      A::Nested(e) => self.compile(e, ctx)?,
      A::Identifier(name) => self.column(name, ctx)?,
      A::CompoundIdentifier(parts)
        if parts.len() == 2 && parts[0].value.eq_ignore_ascii_case("samples") =>
      {
        if ctx.grouped
          && let Some(index) = self.groups.get(&A::Identifier(parts[1].clone()))
        {
          Expr::Group(index)
        } else {
          self.column(&parts[1], ctx)?
        }
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

  fn unary(&mut self, op: &U, expr: &'ast A, ctx: CompileContext) -> Result<Expr> {
    if !matches!(op, U::Not | U::Plus | U::Minus | U::PGBitwiseNot) {
      return Err(format!("unsupported unary operator {op}").into());
    }
    Ok(Expr::Unary(*op, Box::new(self.compile(expr, ctx)?)))
  }

  fn binary(&mut self, left: &'ast A, op: &B, right: &'ast A, ctx: CompileContext) -> Result<Expr> {
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
    expr: &'ast A,
    low: &'ast A,
    high: &'ast A,
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

  fn in_list(
    &mut self,
    expr: &'ast A,
    list: &'ast [A],
    negated: bool,
    ctx: CompileContext,
  ) -> Result<Expr> {
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
    operand: Option<&'ast A>,
    conditions: &'ast [A],
    results: &'ast [A],
    otherwise: Option<&'ast A>,
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

  fn function(&mut self, function: &'ast ast::Function, ctx: CompileContext) -> Result<Expr> {
    let args = function_arguments(function)?;
    let kind = functions::resolve(&function.name);
    // Preserve wildcard diagnostics even for unsupported function names.
    if !matches!(kind, Some(BuiltinFunction::Aggregate(_))) && args.iter().any(Option::is_none) {
      return Err("wildcard requires COUNT(*)".into());
    }
    let kind = kind.ok_or_else(|| {
      format!(
        "unsupported function {}",
        function.name.to_string().to_ascii_lowercase()
      )
    })?;
    let name = kind.canonical_name();
    match kind {
      BuiltinFunction::Aggregate(kind) => self.aggregate(function, name, kind, &args, ctx),
      BuiltinFunction::Temporal(kind) => {
        let args = expression_arguments(args)?;
        self.temporal(function, name, kind, &args, ctx)
      }
      BuiltinFunction::Raw => {
        let args = expression_arguments(args)?;
        self.raw(&args, ctx)
      }
      BuiltinFunction::Scalar(kind) => {
        if !kind.accepts_arity(args.len()) {
          return Err(format!("invalid number of arguments for {name}").into());
        }
        let args = expression_arguments(args)?;
        Ok(Expr::Function(kind, self.arguments(&args, ctx)?))
      }
    }
  }

  fn arguments(&mut self, args: &[&'ast A], ctx: CompileContext) -> Result<Vec<Expr>> {
    args.iter().map(|arg| self.compile(arg, ctx)).collect()
  }

  fn aggregate(
    &mut self,
    function: &'ast ast::Function,
    name: &str,
    kind: AggregateFunction,
    args: &[Option<&'ast A>],
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
    let key = CallKey::new(function, self.aggregate_keys.hasher());
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
    function: &'ast ast::Function,
    name: &str,
    kind: TemporalFunction,
    args: &[&'ast A],
    ctx: CompileContext,
  ) -> Result<Expr> {
    if ctx.grouped {
      return Err("temporal expressions must be grouped or used inside an aggregate".into());
    }
    let arity = kind.expected_arity(args.len());
    if args.len() != arity {
      return Err(format!("{name} requires {arity} arguments").into());
    }
    // Share canonical calls across clauses, checking full AST equality after hashing.
    let key = CallKey::new(function, self.temporal_keys.hasher());
    if let Some(index) = self.temporal_keys.get(&key) {
      return Ok(Expr::Temporal(*index));
    }
    // Lower inputs first so nested temporal dependencies precede this node.
    let args = self.arguments(args, CompileContext::SAMPLE)?;
    if matches!(kind, TemporalFunction::Timeouts)
      && !matches!(args[2], Expr::Const(Cell::Integer(v)) if v >= 0 && v <= i128::from(u64::MAX))
    {
      return Err("timeouts deadline must be a nonnegative u64 integer literal".into());
    }
    let index = self.temporal.len();
    self
      .temporal
      .push(Temporal::new(kind, args, self.max_pending));
    self.temporal_keys.insert(key, index);
    Ok(Expr::Temporal(index))
  }

  fn raw(&mut self, args: &[&'ast A], ctx: CompileContext) -> Result<Expr> {
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

/// Collects non-wildcard arguments while allowing reuse of the input allocation.
fn expression_arguments(args: Vec<Option<&A>>) -> Result<Vec<&A>> {
  args
    .into_iter()
    .collect::<Option<Vec<_>>>()
    .ok_or_else(|| "wildcard requires COUNT(*)".into())
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
    A::Function(f) if functions::is_aggregate(&f.name) => true,
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::sql::normalize::normalize;
  use crate::sql::plan::Plan;

  fn expressions(sql: &str) -> Vec<A> {
    let query = Plan::parse(sql).unwrap();
    let ast::SetExpr::Select(select) = *query.body else {
      panic!()
    };
    let mut expressions = select
      .projection
      .into_iter()
      .map(|item| {
        let ast::SelectItem::UnnamedExpr(expr) = item else {
          panic!()
        };
        expr
      })
      .collect::<Vec<_>>();
    normalize(&mut expressions);
    expressions
  }

  #[test]
  fn canonical_calls_share_slots_without_bypassing_context_checks() {
    let expressions = expressions(
      "SELECT SUM(known(a)), sum(is_known((a))),
      LAG(a), lag((a)), lag(LAG((a))), LaG(lag(a)) FROM samples",
    );
    let schema = HashMap::from([("a".into(), (0, 8))]);
    let mut compiler = Compiler::new(&schema, std::iter::empty(), 10);
    for expr in &expressions {
      compiler
        .compile(expr, CompileContext::projection(false))
        .unwrap();
    }
    assert_eq!(compiler.aggregates.len(), 1);
    assert_eq!(compiler.temporal.len(), 2);
    assert!(
      compiler
        .compile(&expressions[1], CompileContext::SAMPLE)
        .is_err()
    );
    assert!(
      compiler
        .compile(&expressions[3], CompileContext::projection(true))
        .is_err()
    );
  }

  #[test]
  fn hash_collisions_do_not_merge_distinct_calls() {
    let expressions = expressions("SELECT SUM(a), SUM(a + 1) FROM samples");
    let calls = expressions
      .iter()
      .map(|expr| {
        let A::Function(function) = expr else {
          panic!()
        };
        CallKey {
          function,
          fingerprint: 0,
        }
      })
      .collect::<Vec<_>>();
    let indices = HashMap::from([(calls[0], 0), (calls[1], 1)]);
    assert_eq!(indices.len(), 2);
    assert_eq!(indices.get(&calls[0]), Some(&0));
    assert_eq!(indices.get(&calls[1]), Some(&1));
  }
}
