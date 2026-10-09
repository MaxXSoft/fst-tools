//! Conservative expression identity for grouping and shared computations.
//!
//! Parentheses are removed from the tree without changing its operator structure.
//! Only supported, unquoted builtin names and the known/is_known alias are folded.
//! Identifiers, literals, argument order and unsupported modifiers stay intact.

use sqlparser::ast::{Expr, Visit, VisitMut, visit_expressions, visit_expressions_mut};
use std::borrow::Cow;
use std::ops::ControlFlow;

/// Borrows an already canonical tree; otherwise copies and normalizes it once.
/// Keep the original tree for binding resolution, output names and SQL syntax
/// whose meaning depends on outer parentheses (notably ORDER BY ordinals).
pub(super) fn normalized<T: Clone + Visit + VisitMut>(node: &T) -> Cow<'_, T> {
  let changed = visit_expressions(node, |expr| {
    if matches!(expr, Expr::Nested(_)) || canonical_function_name(expr).is_some() {
      ControlFlow::Break(())
    } else {
      ControlFlow::Continue(())
    }
  });
  if changed.is_continue() {
    Cow::Borrowed(node)
  } else {
    let mut node = node.clone();
    normalize(&mut node);
    Cow::Owned(node)
  }
}

/// Normalizes children before their parents, including nested function inputs.
pub(super) fn normalize<T: VisitMut>(node: &mut T) {
  let _ = visit_expressions_mut(node, |expr| {
    if matches!(expr, Expr::Nested(_))
      && let Expr::Nested(inner) = std::mem::replace(expr, Expr::Value(sqlparser::ast::Value::Null))
    {
      *expr = *inner;
    }
    if let Some(name) = canonical_function_name(expr)
      && let Expr::Function(function) = expr
    {
      function.name.0[0].value.clear();
      function.name.0[0].value.push_str(name);
    }
    ControlFlow::<()>::Continue(())
  });
}

/// Returns a replacement only when a supported name actually needs changing.
fn canonical_function_name(expr: &Expr) -> Option<&'static str> {
  let Expr::Function(function) = expr else {
    return None;
  };
  let [name] = function.name.0.as_slice() else {
    return None;
  };
  if name.quote_style.is_some() {
    return None;
  }
  if name.value.eq_ignore_ascii_case("is_known") {
    return Some("known");
  }
  if !name.value.bytes().any(|b| b.is_ascii_uppercase()) {
    return None;
  }
  [
    "count",
    "sum",
    "min",
    "max",
    "coalesce",
    "known",
    "abs",
    "bit",
    "hex",
    "raw",
    "lag",
    "changed",
    "hold",
    "run_length",
    "runs",
    "timeouts",
  ]
  .into_iter()
  .find(|canonical| name.value != *canonical && name.value.eq_ignore_ascii_case(canonical))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::sql::plan::Plan;
  use sqlparser::ast::{SelectItem, SetExpr};

  fn expression(sql: &str) -> Expr {
    let query = Plan::parse(&format!("SELECT {sql} FROM samples"))
      .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let SetExpr::Select(mut select) = *query.body else {
      panic!()
    };
    let SelectItem::UnnamedExpr(expr) = select.projection.remove(0) else {
      panic!()
    };
    expr
  }

  #[test]
  fn canonicalizes_nested_calls_without_mutating_the_source() {
    let original = expression("SuM(Is_KnOwN(((a + (b)))))");
    let before = original.clone();
    let canonical = normalized(&original);
    assert_eq!(*canonical, expression("sum(known(a + b))"));
    assert_eq!(original, before);
    assert!(matches!(normalized(&*canonical), Cow::Borrowed(_)));
  }

  #[test]
  fn keeps_semantic_and_unvalidated_syntax_distinct() {
    for (left, right) in [
      ("a - (b - a)", "(a - b) - a"),
      ("a", "A"),
      ("a", "\"a\""),
      ("a", "samples.a"),
      ("SUM(a)", "\"SUM\"(a)"),
      ("SUM(a)", "SUM(DISTINCT a)"),
      ("SUM(a)", "SUM(a) OVER ()"),
      ("lag(a)", "lag(b)"),
      ("coalesce(a,b)", "coalesce(b,a)"),
      ("'a'", "'A'"),
    ] {
      assert_ne!(
        normalized(&expression(left)),
        normalized(&expression(right)),
        "{left} / {right}"
      );
    }
    let original = expression("SUM(a)");
    let mut filtered = original.clone();
    let Expr::Function(function) = &mut filtered else {
      panic!()
    };
    function.filter = Some(Box::new(Expr::Value(sqlparser::ast::Value::Boolean(true))));
    assert_ne!(normalized(&original), normalized(&filtered));
  }
}
