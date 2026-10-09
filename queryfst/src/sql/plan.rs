//! Query planning and streaming execution of bound SQL expressions.

use crate::error::{Error, Result};
use crate::sql::compiler::{CompileContext, Compiler, has_aggregate, key};
use crate::sql::context::Context as MatchContext;
use crate::sql::ir::{Aggregate, EvalContext, Expr, Order, OrderExpression};
use crate::sql::temporal::Temporal;
use crate::sql::value::{Cell, Column};
use crate::sql::{MatchMode, Options, Report, StopReason};
use sqlparser::ast::{self, Expr as A, GroupByExpr, SelectItem, SetExpr, Statement, TableFactor};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

/// Runtime values for one normalized GROUP BY key tuple.
struct Group {
  /// Normalized keys in the same order as the GROUP BY expressions.
  keys: Vec<Cell>,
  /// Accumulators indexed by Expr::Aggregate references.
  aggregates: Vec<Cell>,
}

/// Projected output retained for sorting or a descending time LIMIT.
struct Row {
  values: Vec<Cell>,
  /// Normalized sort keys, one for each compiled Order rule.
  order: Vec<Cell>,
}

/// Compiled query and its mutable grouping, temporal, and output state.
/// Only validated operations reach execution; a Plan represents a single run.
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

impl Plan {
  pub(super) fn parse(sql: &str) -> Result<Box<ast::Query>> {
    let mut statements = Parser::parse_sql(&GenericDialect {}, sql)?;
    if statements.len() != 1 {
      return Err("exactly one SELECT statement is required".into());
    }
    match statements.remove(0) {
      Statement::Query(query) => Ok(query),
      _ => Err("only SELECT queries are supported".into()),
    }
  }

  pub(super) fn compile(
    query: &ast::Query,
    schema: &HashMap<String, (usize, u32)>,
    options: &Options,
  ) -> Result<Self> {
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
    let mut compiler = Compiler::new(schema, group_ast, options.max_buffer_rows.unwrap_or(100000));
    let selection = select
      .selection
      .as_ref()
      .map(|expr| compiler.compile(expr, CompileContext::SAMPLE))
      .transpose()?;
    let group_by = group_ast
      .iter()
      .map(|expr| compiler.compile(expr, CompileContext::SAMPLE))
      .collect::<Result<Vec<_>>>()?;
    let projection = projection_ast
      .iter()
      .map(|expr| compiler.compile(expr, CompileContext::projection(aggregate)))
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
          None => OrderExpression::Expression(
            compiler.compile(&item.expr, CompileContext::projection(aggregate))?,
          ),
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
      let ctx = EvalContext {
        cells,
        temporal: &self.temporal_values,
        groups: &[],
        aggregates: &[],
      };
      self.temporal_values[i] = match self.temporal[i].advance(&ctx) {
        Ok(value) => value,
        Err(Error::PendingRequestBudget) => {
          report.stop_reason = Some(StopReason::PendingRequestBudget);
          return Ok(false);
        }
        Err(error) => return Err(error),
      };
    }
    let ctx = EvalContext {
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
      if let Some(deadline) = temporal.deadline() {
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
        let ctx = EvalContext {
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
