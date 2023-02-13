mod capi;
mod consts;
mod types;
mod utils;
mod writer;

pub use consts::*;
pub use types::*;
pub use writer::*;

use std::fmt;

/// Error that may returned from FST-related APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
  /// CString conversion error.
  CStringConv(std::ffi::NulError),
  /// Writer creation error.
  WriterCreate,
  /// Null handle.
  NullHandle,
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::CStringConv(e) => write!(f, "CString conversion error, {e}"),
      Self::WriterCreate => write!(f, "writer creation error"),
      Self::NullHandle => write!(f, "null handle"),
    }
  }
}

/// Result that error type is [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
