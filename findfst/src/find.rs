use regex::{bytes::Regex, Error as RegexError};
use std::fmt;

/// Errors that can occurr when constructing [`MatchInfo`].
pub enum Error {
  Regex(RegexError),
  InvalidHex(String),
  InvalidBin(String),
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Regex(e) => write!(f, "Invalid value regex: {e}"),
      Self::InvalidHex(v) => write!(f, "Invalid hexadecimal value: {v}!"),
      Self::InvalidBin(v) => write!(f, "Invalid binary value: {v}!"),
    }
  }
}

/// Information for matching values.
pub enum MatchInfo {
  Regex(Regex, bool),
  Exact(Box<[u8]>),
}

impl MatchInfo {
  pub fn new(value: String, hex: bool, regex: bool) -> Result<Self, Error> {
    if regex {
      let re = Regex::new(&value).map_err(Error::Regex)?;
      Ok(Self::Regex(re, hex))
    } else if hex {
      let mut s = Vec::new();
      for c in value.chars() {
        let digit = match c.to_digit(16) {
          Some(d) => d,
          _ => return Err(Error::InvalidHex(value)),
        };
        s.push(if (digit & 8) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 4) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 2) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 1) != 0 { b'1' } else { b'0' });
      }
      Ok(Self::Exact(s.into()))
    } else if value.contains(|c: char| !c.is_digit(2)) {
      Err(Error::InvalidBin(value))
    } else {
      Ok(Self::Exact(value.into_bytes().into()))
    }
  }
}
