use regex::bytes::Regex;
use std::iter;

/// Trait for matching values in different configurations.
pub trait ValueMatcher {
  fn is_match(&self, value: &[u8]) -> bool;
}

/// Use regex to match binary values.
pub struct RegexMatcher {
  re: Regex,
}

impl RegexMatcher {
  pub fn new(re: Regex) -> Self {
    Self { re }
  }
}

impl ValueMatcher for RegexMatcher {
  fn is_match(&self, value: &[u8]) -> bool {
    self.re.is_match(value)
  }
}

/// Use regex to match hexadecimal values.
pub struct RegexHexMatcher {
  re: Regex,
}

impl RegexHexMatcher {
  pub fn new(re: Regex) -> Self {
    Self { re }
  }
}

impl ValueMatcher for RegexHexMatcher {
  fn is_match(&self, value: &[u8]) -> bool {
    let hex = value
      .rchunks(4)
      .rev()
      .map(|ds| {
        let digit = ds
          .iter()
          .fold(0, |ans, d| (ans << 1) | ((*d != b'0') as u32));
        char::from_digit(digit, 16).unwrap() as u8
      })
      .collect::<Vec<_>>();
    self.re.is_match(&hex)
  }
}

/// Use byte array to match any value.
pub struct ExactMatcher {
  exact: Box<[u8]>,
}

impl ExactMatcher {
  pub fn new(exact: Box<[u8]>) -> Self {
    Self { exact }
  }
}

impl ValueMatcher for ExactMatcher {
  fn is_match(&self, value: &[u8]) -> bool {
    if value.len() > self.exact.len() {
      value
        .iter()
        .rev()
        .zip(self.exact.iter().rev().chain(iter::repeat(&b'0')))
        .all(|(l, r)| l == r)
    } else if value.len() < self.exact.len() {
      value
        .iter()
        .rev()
        .chain(iter::repeat(&b'0'))
        .zip(self.exact.iter().rev())
        .all(|(l, r)| l == r)
    } else {
      value
        .iter()
        .rev()
        .zip(self.exact.iter().rev())
        .all(|(l, r)| l == r)
    }
  }
}
