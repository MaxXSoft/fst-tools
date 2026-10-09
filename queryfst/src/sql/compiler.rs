//! Validation and lowering of SQL AST expressions into bound expressions.

use crate::error::Result;
use crate::sql::deadline::Deadline;
use crate::sql::ir::{Aggregate, Expr};
use crate::sql::temporal::Temporal;
use crate::sql::value::Cell;
use sqlparser::ast::{
  self, BinaryOperator as B, Expr as A, FunctionArg, FunctionArgExpr, FunctionArguments,
  UnaryOperator as U,
};
use std::collections::HashMap;

/// Resolves sample bindings and interns aggregate and temporal expressions.
pub(super) struct Compiler<'a> {
  schema: &'a HashMap<String, (usize, u32)>,
  groups: Vec<String>,
  max_pending: usize,
  pub aggregates: Vec<Aggregate>,
  aggregate_keys: HashMap<String, usize>,
  pub temporal: Vec<Temporal>,
  temporal_keys: HashMap<String, usize>,
}

/// Textual GROUP BY identity, ignoring outer parentheses only.
pub(super) fn key(expr: &A) -> String {
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

  pub(super) fn compile(&mut self, expr: &A, aggregate: bool, grouped: bool) -> Result<Expr> {
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

pub(super) fn has_aggregate(expr: &A) -> bool {
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
