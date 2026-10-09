//! Shared SQL scalar values and output column metadata.

use crate::error::Result;

/// A lossless scalar value used by SQL expressions and output rows.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cell {
  /// SQL NULL, including an unavailable sampled value.
  Null,
  /// Predicate result; numeric conversion maps false/true to zero/one.
  Bool(bool),
  /// Exact signed integer used for arithmetic and known narrow logic.
  Integer(i128),
  /// SQL text literal or explicitly formatted value.
  Text(String),
  /// Original logic bits containing unknown states or more than 127 bits.
  /// Known bits can still be converted if their value fits a signed i128.
  Bits(String),
}

impl Cell {
  /// Serializes integers as decimal strings, retaining four-state/wide bits.
  pub fn to_json(&self) -> serde_json::Value {
    match self {
      Self::Null => serde_json::Value::Null,
      Self::Bool(v) => (*v).into(),
      Self::Integer(v) => v.to_string().into(),
      Self::Text(v) => v.clone().into(),
      Self::Bits(v) => serde_json::json!({"bits": v}),
    }
  }

  /// Converts known logic to integers while retaining unknown/wide values.
  pub(super) fn logic(bytes: &[u8]) -> Result<Self> {
    if bytes.iter().all(|c| matches!(c, b'0' | b'1')) && bytes.len() <= 127 {
      Ok(Self::Integer(
        bytes.iter().fold(0, |v, c| (v << 1) | i128::from(c - b'0')),
      ))
    } else if bytes.iter().all(|c| {
      matches!(
        c.to_ascii_lowercase(),
        b'0' | b'1' | b'x' | b'z' | b'h' | b'l' | b'u' | b'w' | b'-'
      )
    }) {
      Ok(Self::Bits(std::str::from_utf8(bytes)?.into()))
    } else {
      Err("SQL sampling currently accepts fixed-width logic signals only".into())
    }
  }

  /// Numeric operators propagate unknown values; oversized known values error.
  pub(super) fn integer(&self) -> Result<Option<i128>> {
    match self {
      Self::Null => Ok(None),
      Self::Bool(v) => Ok(Some(i128::from(*v))),
      Self::Integer(v) => Ok(Some(*v)),
      Self::Bits(bits) if bits.bytes().all(|b| matches!(b, b'0' | b'1')) => {
        i128::from_str_radix(bits, 2).map(Some).map_err(|_| {
          "numeric operation exceeds signed 128-bit range; use raw() for wide bits".into()
        })
      }
      Self::Bits(_) => Ok(None),
      Self::Text(_) => Err("numeric operation on text".into()),
    }
  }

  /// Canonicalizes SQL-equivalent known numeric keys without losing unknown bits.
  pub(super) fn normalized(self) -> Result<Self> {
    match &self {
      Self::Bool(value) => Ok(Self::Integer(i128::from(*value))),
      Self::Bits(bits) if bits.bytes().all(|b| matches!(b, b'0' | b'1')) => {
        Ok(Self::Integer(self.integer()?.unwrap()))
      }
      _ => Ok(self),
    }
  }

  /// SQL predicates use three-valued truth, with integer zero treated as false.
  pub(super) fn truth(&self) -> Result<Option<bool>> {
    Ok(self.integer()?.map(|v| v != 0))
  }
}

/// Named output expression.
#[derive(Clone, Debug)]
pub struct Column {
  /// SELECT alias or expression text used in output headers.
  pub name: String,
}
