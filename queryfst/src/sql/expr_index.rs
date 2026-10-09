//! First-occurrence lookup for normalized GROUP BY and SELECT expressions.

use sqlparser::ast::Expr;
use std::collections::HashMap;

/// Borrows canonical AST nodes for the duration of planning. Small lists avoid
/// hashing and table construction; large lists avoid repeated linear searches.
/// Both representations retain the first index of duplicate expressions.
pub(super) enum ExprIndex<'a> {
  Linear(Vec<&'a Expr>),
  Hashed(HashMap<&'a Expr, usize>),
}

impl<'a> ExprIndex<'a> {
  /// The SQL planning microbench favors linear lookup through 16 expressions.
  pub(super) fn new(expressions: impl ExactSizeIterator<Item = &'a Expr>) -> Self {
    if expressions.len() <= 16 {
      Self::Linear(expressions.collect())
    } else {
      let mut indices = HashMap::with_capacity(expressions.len());
      for (index, expr) in expressions.enumerate() {
        indices.entry(expr).or_insert(index);
      }
      Self::Hashed(indices)
    }
  }

  pub(super) fn get(&self, expr: &Expr) -> Option<usize> {
    match self {
      Self::Linear(expressions) => expressions.iter().position(|candidate| *candidate == expr),
      Self::Hashed(indices) => indices.get(expr).copied(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use sqlparser::ast::Value;

  #[test]
  fn both_representations_keep_the_first_duplicate_index() {
    for count in [4, 32] {
      let mut expressions = (0..count)
        .map(|i| Expr::Value(Value::Number(i.to_string(), false)))
        .collect::<Vec<_>>();
      expressions.push(expressions[0].clone());
      let index = ExprIndex::new(expressions.iter());
      assert_eq!(index.get(&expressions[count]), Some(0));
      assert_eq!(index.get(&expressions[count - 1]), Some(count - 1));
      assert_eq!(index.get(&Expr::Value(Value::Null)), None);
    }
  }
}
