use std::{error, fmt, io};

/// Preserve error sources and distinguish invalid CLI usage from execution errors.
#[derive(Debug)]
pub(crate) enum Error {
  Arguments(String),
  Fst(fstapi::Error),
  Io(io::Error),
  Output(io::Error),
  SqlParse(sqlparser::parser::ParserError),
  Json(serde_json::Error),
  Query(String),
  Encoding(Box<dyn error::Error>),
  PendingRequestBudget,
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Arguments(message) | Self::Query(message) => f.write_str(message),
      Self::Fst(error) => fmt::Display::fmt(error, f),
      Self::Io(error) | Self::Output(error) => fmt::Display::fmt(error, f),
      Self::SqlParse(error) => fmt::Display::fmt(error, f),
      Self::Json(error) => fmt::Display::fmt(error, f),
      Self::Encoding(error) => fmt::Display::fmt(error, f),
      Self::PendingRequestBudget => f.write_str("pending request budget exhausted"),
    }
  }
}

impl error::Error for Error {
  fn source(&self) -> Option<&(dyn error::Error + 'static)> {
    match self {
      Self::Fst(error) => Some(error),
      Self::Io(error) | Self::Output(error) => Some(error),
      Self::SqlParse(error) => Some(error),
      Self::Json(error) => Some(error),
      Self::Encoding(error) => Some(error.as_ref()),
      _ => None,
    }
  }
}

impl Error {
  pub(crate) fn code(&self) -> &'static str {
    match self {
      Self::Arguments(_) => "invalid_arguments",
      Self::Fst(_) | Self::Io(_) => "input_error",
      Self::Json(_) | Self::Encoding(_) => "encoding_error",
      Self::Output(_) => "output_error",
      Self::Query(_) | Self::SqlParse(_) | Self::PendingRequestBudget => "query_error",
    }
  }

  pub(crate) fn exit_code(&self) -> i32 {
    if matches!(self, Self::Arguments(_)) {
      2
    } else {
      1
    }
  }
}

impl From<fstapi::Error> for Error {
  fn from(error: fstapi::Error) -> Self {
    Self::Fst(error)
  }
}
impl From<io::Error> for Error {
  fn from(error: io::Error) -> Self {
    Self::Io(error)
  }
}
impl From<serde_json::Error> for Error {
  fn from(error: serde_json::Error) -> Self {
    Self::Json(error)
  }
}
impl From<String> for Error {
  fn from(message: String) -> Self {
    Self::Query(message)
  }
}
impl From<&str> for Error {
  fn from(message: &str) -> Self {
    Self::Query(message.into())
  }
}
impl From<std::str::Utf8Error> for Error {
  fn from(error: std::str::Utf8Error) -> Self {
    Self::Encoding(Box::new(error))
  }
}
impl From<std::array::TryFromSliceError> for Error {
  fn from(error: std::array::TryFromSliceError) -> Self {
    Self::Encoding(Box::new(error))
  }
}
impl From<sqlparser::parser::ParserError> for Error {
  fn from(error: sqlparser::parser::ParserError) -> Self {
    Self::SqlParse(error)
  }
}
