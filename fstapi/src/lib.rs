mod capi;
mod consts;
mod reader;
mod types;
mod utils;
mod writer;

pub use consts::*;
pub use reader::*;
pub use types::*;
pub use writer::*;

use std::fmt;

/// Error that may returned from FST-related APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
  /// Invalid UTF-8 string.
  InvalidUtf8Str(Option<std::str::Utf8Error>),
  /// CString conversion error.
  CStringConv(std::ffi::NulError),
  /// Context creation error.
  ContextCreate,
  /// Invalid operation.
  InvalidOperation,
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::InvalidUtf8Str(None) => write!(f, "invalid UTF-8 string"),
      Self::InvalidUtf8Str(Some(e)) => write!(f, "{e}"),
      Self::CStringConv(e) => write!(f, "CString conversion error, {e}"),
      Self::ContextCreate => write!(f, "context creation error"),
      Self::InvalidOperation => write!(f, "invalid operation"),
    }
  }
}

/// Result that error type is [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
