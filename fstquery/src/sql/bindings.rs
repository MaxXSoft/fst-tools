//! Resolve declarations once, with one state slot per SQL binding.

use crate::error::Result;
use fstapi::{Handle, Reader};
use sqlparser::ast::{self, Expr, SelectItem, SetExpr, visit_expressions};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::ControlFlow;

pub(super) struct ResolvedSignals {
  pub schema: HashMap<String, (usize, u32)>,
  pub mapping: HashMap<Handle, Vec<usize>>,
  pub tick_index: usize,
  pub sample_index: usize,
}

pub(super) fn resolve(
  reader: &mut Reader,
  bindings: &BTreeMap<String, String>,
  query: &ast::Query,
) -> Result<ResolvedSignals> {
  let mut bindings = bindings.clone();
  for name in quoted_paths(query) {
    // Explicit aliases and the two virtual columns take precedence.
    if !matches!(name.as_str(), "tick" | "sample_index") {
      bindings.entry(name.clone()).or_insert(name);
    }
  }
  let names: Vec<_> = bindings.keys().cloned().collect();
  if names
    .iter()
    .any(|n| matches!(n.as_str(), "tick" | "sample_index"))
  {
    return Err("binding names tick and sample_index are reserved".into());
  }
  let mut paths: HashMap<&str, Vec<usize>> = HashMap::new();
  for (i, name) in names.iter().enumerate() {
    paths.entry(bindings[name].as_str()).or_default().push(i);
  }
  let mut mapping = HashMap::new();
  let mut widths = vec![0; names.len()];
  let mut found = vec![false; names.len()];
  for entry in reader.vars() {
    let (path, var) = entry?;
    if let Some(indices) = paths.get(path.as_str()) {
      if indices.iter().any(|&i| found[i]) {
        return Err(
          format!("ambiguous signal path {path:?}: multiple declarations have this full name")
            .into(),
        );
      }
      if var.length() == 0
        || matches!(
          var.ty(),
          fstapi::var_type::GEN_STRING
            | fstapi::var_type::VCD_EVENT
            | fstapi::var_type::VCD_REAL
            | fstapi::var_type::VCD_REAL_PARAMETER
            | fstapi::var_type::VCD_REALTIME
            | fstapi::var_type::SV_SHORTREAL
            | fstapi::var_type::VCD_PORT
        )
      {
        return Err(format!("binding {path} is not fixed-width logic").into());
      }
      for &i in indices {
        widths[i] = var.length();
        found[i] = true;
      }
      mapping
        .entry(var.handle())
        .or_insert_with(Vec::new)
        .extend(indices.iter().copied());
    }
  }
  if found.iter().any(|v| !v) {
    return Err(
      format!(
        "missing bound signals: {}",
        names
          .iter()
          .enumerate()
          .filter(|(i, _)| !found[*i])
          .map(|(_, name)| format!("{name:?} -> {:?}", bindings[name]))
          .collect::<Vec<_>>()
          .join(", ")
      )
      .into(),
    );
  }
  let tick_index = names.len();
  let sample_index = tick_index + 1;
  let schema: HashMap<_, _> = names
    .iter()
    .enumerate()
    .map(|(i, name)| (name.clone(), (i, widths[i])))
    .chain([
      ("tick".into(), (tick_index, 64)),
      ("sample_index".into(), (sample_index, 64)),
    ])
    .collect();
  Ok(ResolvedSignals {
    schema,
    mapping,
    tick_index,
    sample_index,
  })
}

/// Collect expression references from the parsed AST, never from SQL text.
/// ORDER BY output aliases are resolved by the planner before input columns.
fn quoted_paths(query: &ast::Query) -> BTreeSet<String> {
  let SetExpr::Select(select) = &*query.body else {
    return BTreeSet::new();
  };
  let mut names = BTreeSet::new();
  let mut collect = |expr: &Expr| {
    let name = match expr {
      Expr::Identifier(name) => Some(name),
      Expr::CompoundIdentifier(parts)
        if parts.len() == 2 && parts[0].value.eq_ignore_ascii_case("samples") =>
      {
        Some(&parts[1])
      }
      _ => None,
    };
    if let Some(name) = name.filter(|name| name.quote_style.is_some()) {
      names.insert(name.value.clone());
    }
    ControlFlow::<()>::Continue(())
  };
  let _ = visit_expressions(&select.projection, &mut collect);
  let _ = visit_expressions(&select.selection, &mut collect);
  let _ = visit_expressions(&select.group_by, &mut collect);
  let _ = visit_expressions(&select.having, &mut collect);
  if let Some(order) = &query.order_by {
    for item in &order.exprs {
      let projected = if let Expr::Identifier(name) = &item.expr {
        select.projection.iter().any(|item| match item {
          SelectItem::ExprWithAlias { alias, .. } => alias.value == name.value,
          SelectItem::UnnamedExpr(expr) => expr.to_string() == name.value,
          _ => false,
        })
      } else {
        false
      };
      if !projected {
        let _ = visit_expressions(&item.expr, &mut collect);
      }
    }
  }
  names
}
