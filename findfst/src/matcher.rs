use regex::bytes::Regex;
use std::cmp::Ordering;
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
        let digit = ds.iter().try_fold(0, |ans, d| match d {
          b'0' => Some(ans << 1),
          b'1' => Some((ans << 1) | 1),
          _ => None,
        })?;
        Some(char::from_digit(digit, 16).unwrap() as u8)
      })
      .collect::<Option<Vec<_>>>();
    // Only known binary values have a numeric hexadecimal representation.
    // Skip unknown/high-impedance states and nonbinary string/real values.
    hex.is_some_and(|hex| self.re.is_match(&hex))
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
    match value.len().cmp(&self.exact.len()) {
      Ordering::Greater => value
        .iter()
        .rev()
        .zip(self.exact.iter().rev().chain(iter::repeat(&b'0')))
        .all(|(l, r)| l == r),
      Ordering::Less => value
        .iter()
        .rev()
        .chain(iter::repeat(&b'0'))
        .zip(self.exact.iter().rev())
        .all(|(l, r)| l == r),
      Ordering::Equal => value
        .iter()
        .rev()
        .zip(self.exact.iter().rev())
        .all(|(l, r)| l == r),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hex_regex_preserves_partial_nibbles_and_rejects_nonbinary_values() {
    let partial = RegexHexMatcher::new(Regex::new("^1f$").unwrap());
    assert!(partial.is_match(b"11111"));
    let all = RegexHexMatcher::new(Regex::new(".*").unwrap());
    for value in [&b"0x01"[..], b"1z11", b"XXXX", b"ZZZZ", b"1.25", b"\xff"] {
      assert!(!all.is_match(value), "{value:?}");
    }
  }
}
