//! Coalesce sample windows around all, the first, or the last WHERE match.
use super::{Cell, MatchMode};
use crate::error::Result;
use std::collections::VecDeque;

pub(super) struct Context {
  before: usize,
  after: u64,
  remaining: u64,
  mode: MatchMode,
  seen: bool,
  history: VecDeque<(u64, Vec<Cell>, bool)>,
  last_window: Vec<Vec<Cell>>,
  last_emitted: Option<u64>,
  pub before_clipped: bool,
}

impl Context {
  pub fn new(before: u64, after: u64, capacity: usize, mode: MatchMode) -> Result<Self> {
    let before =
      usize::try_from(before).map_err(|_| "context before count exceeds address space")?;
    // Last mode retains both the rolling history and the candidate window.
    let required = if mode == MatchMode::Last {
      (before as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add(after))
        .and_then(|n| n.checked_add(1))
        .ok_or("context size overflow")?
    } else {
      before as u64
    };
    if required > capacity as u64 {
      return Err("context storage exceeds --max-buffer-rows".into());
    }
    Ok(Self {
      before,
      after,
      remaining: 0,
      mode,
      seen: false,
      history: VecDeque::new(),
      last_window: Vec::new(),
      last_emitted: None,
      before_clipped: false,
    })
  }

  pub fn pending_after(&self) -> u64 {
    self.remaining
  }
  pub fn done(&self) -> bool {
    self.mode == MatchMode::First && self.seen && self.remaining == 0
  }
  pub fn finish(&mut self) -> Vec<Vec<Cell>> {
    std::mem::take(&mut self.last_window)
  }

  /// Adds `__match`; overlapping all-match windows never duplicate samples.
  /// In first/last mode this flag identifies the selected trigger only.
  pub fn push(&mut self, index: u64, row: Vec<Cell>, matched: bool) -> Vec<Vec<Cell>> {
    let matched = matched && !(self.mode == MatchMode::First && self.seen);
    let mut result = Vec::new();
    if self.mode == MatchMode::Last {
      if matched {
        self.before_clipped = index < self.before as u64;
        self.last_window.clear();
        for (_, value, _) in &self.history {
          let mut value = value.clone();
          value.push(Cell::Bool(false));
          self.last_window.push(value);
        }
      }
      if matched || self.remaining > 0 {
        let mut value = row.clone();
        value.push(Cell::Bool(matched));
        self.last_window.push(value);
      }
    } else {
      if matched {
        self.before_clipped |= index < self.before as u64;
        for (prior, value, is_match) in &self.history {
          if self.last_emitted.is_none_or(|last| *prior > last) {
            let mut value = value.clone();
            value.push(Cell::Bool(*is_match));
            result.push(value);
            self.last_emitted = Some(*prior);
          }
        }
      }
      if matched || self.remaining > 0 {
        let mut value = row.clone();
        value.push(Cell::Bool(matched));
        result.push(value);
        self.last_emitted = Some(index);
      }
    }
    self.seen |= matched;
    self.remaining = if matched {
      self.after
    } else {
      self.remaining.saturating_sub(1)
    };
    if self.before > 0 {
      if self.history.len() == self.before {
        self.history.pop_front();
      }
      self.history.push_back((index, row, matched));
    }
    result
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn overlapping_windows_emit_once_and_expose_clipped_tail() {
    let mut context = Context::new(2, 2, 10, MatchMode::All).unwrap();
    let mut output = Vec::new();
    for index in 0..7 {
      output.extend(context.push(
        index,
        vec![Cell::Integer(index.into())],
        index == 2 || index == 4,
      ));
    }
    assert_eq!(output.len(), 7);
    for (index, row) in output.iter().enumerate() {
      assert_eq!(row[0], Cell::Integer(index as i128));
      assert_eq!(row[1], Cell::Bool(index == 2 || index == 4));
    }
    assert_eq!(context.pending_after(), 0);
    assert!(!context.before_clipped);
    context.push(7, vec![Cell::Integer(7)], true);
    assert_eq!(context.pending_after(), 2);
  }
  #[test]
  fn first_and_last_select_triggers_before_expanding_context() {
    let mut first = Context::new(1, 2, 10, MatchMode::First).unwrap();
    let mut last = Context::new(1, 2, 10, MatchMode::Last).unwrap();
    let mut output = Vec::new();
    for index in 0..8 {
      let row = vec![Cell::Integer(index.into())];
      if !first.done() {
        output.extend(first.push(index, row.clone(), index == 2 || index == 4));
      }
      assert!(last.push(index, row, index == 2 || index == 4).is_empty());
    }
    assert!(first.done());
    assert_eq!(
      output.iter().map(|r| r[0].clone()).collect::<Vec<_>>(),
      (1..=4).map(Cell::Integer).collect::<Vec<_>>()
    );
    assert_eq!(
      last
        .finish()
        .iter()
        .map(|r| r[0].clone())
        .collect::<Vec<_>>(),
      (3..=6).map(Cell::Integer).collect::<Vec<_>>()
    );
    assert!(!last.done());
  }
}
