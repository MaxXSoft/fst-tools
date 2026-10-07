//! Bounded request/response tracking for `timeouts(req, response, cycles [, key])`.
//! Responses at the deadline sample satisfy the oldest outstanding request of
//! the same key. Pending requests at EOF are unresolved, never counted as late.
use super::Cell;
use crate::error::{Error, Result};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug)]
pub(super) struct Deadline {
  pending: BTreeMap<Cell, VecDeque<(u64, bool)>>,
  sample: u64,
  count: usize,
  limit: usize,
  cycles: Option<u64>,
  pub invalidated: u64,
  pub unmatched_responses: u64,
}

impl Deadline {
  pub fn new(limit: usize) -> Self {
    Self {
      pending: BTreeMap::new(),
      sample: 0,
      count: 0,
      limit,
      cycles: None,
      invalidated: 0,
      unmatched_responses: 0,
    }
  }

  pub fn pending(&self) -> u64 {
    self
      .pending
      .values()
      .map(|q| q.iter().filter(|(_, expired)| !expired).count() as u64)
      .sum()
  }

  pub fn step(&mut self, args: &[Cell]) -> Result<Cell> {
    if !(3..=4).contains(&args.len()) {
      return Err("timeouts requires request, response, deadline cycles, and optional key".into());
    }
    let cycles = args[2]
      .integer()?
      .ok_or("timeouts deadline must be a known nonnegative integer")?;
    let cycles = u64::try_from(cycles).map_err(|_| "timeouts deadline must fit u64")?;
    if self.cycles.is_some_and(|n| n != cycles) {
      return Err("timeouts deadline must remain constant".into());
    }
    self.cycles = Some(cycles);
    let sample = self.sample;
    self.sample = self
      .sample
      .checked_add(1)
      .ok_or("timeouts sample counter overflow")?;
    let req = args[0].truth()?;
    let resp = args[1].truth()?;
    let key = args.get(3).cloned().unwrap_or(Cell::Integer(0));
    let key_unknown = matches!(key, Cell::Null | Cell::Bits(_));
    if req.is_none() || resp.is_none() || (key_unknown && (req == Some(true) || resp == Some(true)))
    {
      self.invalidated += self.pending();
      self.pending.clear();
      self.count = 0;
      return Ok(Cell::Null);
    }
    // Consume an old response before accepting the new request. When no older
    // request exists, a same-sample request/response completes immediately.
    let mut immediate = false;
    if resp == Some(true) {
      let found = self
        .pending
        .get_mut(&key)
        .is_some_and(|q| q.pop_front().is_some());
      if found {
        self.count -= 1;
      } else if req == Some(true) {
        immediate = true;
      } else {
        self.unmatched_responses += 1;
      }
    }
    if req == Some(true) && !immediate {
      if self.count >= self.limit {
        return Err(Error::PendingRequestBudget);
      }
      self
        .pending
        .entry(key.clone())
        .or_default()
        .push_back((sample, false));
      self.count += 1;
    }
    let mut expired = 0u64;
    self.pending.retain(|_, requests| {
      for (time, already_expired) in requests.iter_mut() {
        if !*already_expired && sample - *time >= cycles {
          *already_expired = true;
          expired += 1;
        }
      }
      !requests.is_empty()
    });
    // Keep expired entries as bounded FIFO tombstones: a late response must not
    // accidentally satisfy a newer request. They also consume the state budget.
    Ok(Cell::Integer(i128::from(expired)))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  fn step(d: &mut Deadline, req: bool, resp: bool, n: i128, key: i128) -> Cell {
    d.step(&[
      Cell::Bool(req),
      Cell::Bool(resp),
      Cell::Integer(n),
      Cell::Integer(key),
    ])
    .unwrap()
  }
  #[test]
  fn inclusive_deadline_and_unresolved_eof() {
    let mut d = Deadline::new(10);
    assert_eq!(step(&mut d, true, false, 2, 0), Cell::Integer(0));
    assert_eq!(d.pending(), 1); // EOF here would be unresolved.
    assert_eq!(step(&mut d, false, false, 2, 0), Cell::Integer(0));
    assert_eq!(step(&mut d, false, true, 2, 0), Cell::Integer(0));
    assert_eq!(d.pending(), 0);
    step(&mut d, true, false, 2, 0);
    step(&mut d, false, false, 2, 0);
    assert_eq!(step(&mut d, false, false, 2, 0), Cell::Integer(1));
  }
  #[test]
  fn keyed_fifo_unknowns_and_zero_deadline() {
    let mut d = Deadline::new(4);
    step(&mut d, true, false, 2, 1);
    step(&mut d, true, true, 2, 2);
    assert_eq!(step(&mut d, false, false, 2, 0), Cell::Integer(1));
    let mut d = Deadline::new(4);
    assert_eq!(step(&mut d, true, true, 0, 0), Cell::Integer(0));
    assert_eq!(step(&mut d, true, false, 0, 0), Cell::Integer(1));
    let mut d = Deadline::new(4);
    step(&mut d, true, false, 9, 0);
    assert_eq!(
      d.step(&[Cell::Null, Cell::Bool(false), Cell::Integer(9)])
        .unwrap(),
      Cell::Null
    );
    assert_eq!(d.invalidated, 1);
    assert_eq!(d.pending(), 0);
  }
  #[test]
  fn late_response_does_not_satisfy_a_newer_request() {
    let mut d = Deadline::new(4);
    step(&mut d, true, false, 2, 0);
    step(&mut d, true, false, 2, 0);
    assert_eq!(step(&mut d, false, false, 2, 0), Cell::Integer(1));
    // This response belongs to the already-expired request from sample zero.
    assert_eq!(step(&mut d, false, true, 2, 0), Cell::Integer(1));
    assert_eq!(d.pending(), 0);
    assert_eq!(d.unmatched_responses, 0);
  }
  #[test]
  fn response_frees_capacity_for_same_sample_request() {
    let mut d = Deadline::new(1);
    step(&mut d, true, false, 2, 0);
    assert_eq!(step(&mut d, true, true, 2, 0), Cell::Integer(0));
    assert_eq!(d.pending(), 1);
  }
}
