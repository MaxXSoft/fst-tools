//! Bounded request/response tracking for `timeouts(req, response, cycles [, key])`.
//! Responses at the deadline sample satisfy the oldest outstanding request of
//! the same key. Pending requests at EOF are unresolved, never counted as late.

use crate::error::{Error, Result};
use crate::sql::value::Cell;
use std::collections::{BTreeMap, VecDeque};

/// Known request IDs, with numeric equality independent of logic width.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
  Integer(i128),
  Text(String),
  WideBits(String),
}

impl Key {
  fn from_cell(value: &Cell) -> Option<Self> {
    match value {
      Cell::Null => None,
      Cell::Bool(value) => Some(Self::Integer(i128::from(*value))),
      Cell::Integer(value) => Some(Self::Integer(*value)),
      Cell::Text(value) => Some(Self::Text(value.clone())),
      Cell::Bits(bits) if bits.bytes().all(|b| matches!(b, b'0' | b'1')) => {
        let bits = bits.trim_start_matches('0');
        Some(if bits.is_empty() {
          Self::Integer(0)
        } else {
          i128::from_str_radix(bits, 2)
            .map(Self::Integer)
            .unwrap_or_else(|_| Self::WideBits(bits.into()))
        })
      }
      Cell::Bits(_) => None,
    }
  }
}

#[derive(Debug)]
pub(super) struct Deadline {
  pending: BTreeMap<Key, VecDeque<(u64, bool)>>,
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
    let key = Key::from_cell(args.get(3).unwrap_or(&Cell::Integer(0)));
    if req.is_none()
      || resp.is_none()
      || (key.is_none() && (req == Some(true) || resp == Some(true)))
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
        .get_mut(key.as_ref().unwrap())
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
        .entry(key.unwrap())
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

  #[test]
  fn keys_match_across_numeric_representations_and_leading_zeroes() {
    let wide = "1".repeat(256);
    for (request, response) in [
      (Cell::Bits("0".repeat(128)), Cell::Integer(0)),
      (
        Cell::Bits(format!("{}1", "0".repeat(127))),
        Cell::Bool(true),
      ),
      (Cell::Bool(true), Cell::Integer(1)),
      (Cell::Bits(format!("000{wide}")), Cell::Bits(wide)),
    ] {
      let mut d = Deadline::new(4);
      for (req, resp, key) in [(true, false, request), (false, true, response)] {
        assert_eq!(
          d.step(&[Cell::Bool(req), Cell::Bool(resp), Cell::Integer(1), key])
            .unwrap(),
          Cell::Integer(0)
        );
      }
      assert_eq!(d.pending(), 0);
      assert_eq!(d.invalidated, 0);
      assert_eq!(d.unmatched_responses, 0);
    }
  }

  #[test]
  fn distinct_wide_and_text_keys_do_not_satisfy_other_requests() {
    let wide = format!("1{}", "0".repeat(255));
    for response in [Cell::Bits("1".repeat(256)), Cell::Text(wide.clone())] {
      let mut d = Deadline::new(4);
      d.step(&[
        Cell::Bool(true),
        Cell::Bool(false),
        Cell::Integer(1),
        Cell::Bits(wide.clone()),
      ])
      .unwrap();
      assert_eq!(
        d.step(&[
          Cell::Bool(false),
          Cell::Bool(true),
          Cell::Integer(1),
          response
        ])
        .unwrap(),
        Cell::Integer(1)
      );
      assert_eq!(d.unmatched_responses, 1);
      assert_eq!(d.invalidated, 0);
    }
  }
}
