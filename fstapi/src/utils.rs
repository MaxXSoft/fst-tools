use crate::{Error, Result};
use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::Path;

/// Trait for converting [`Path`] into string.
pub(crate) trait ToStr<'a> {
  /// Converts to <code>&[str]</code>.
  fn to_str(&'a self) -> Result<&'a str>;
}

impl<'a, P> ToStr<'a> for P
where
  P: AsRef<Path>,
{
  fn to_str(&'a self) -> Result<&'a str> {
    self.as_ref().to_str().ok_or(Error::InvalidUtf8Str)
  }
}

/// Trait for converting bytes into [`CString`].
pub(crate) trait IntoCString {
  /// Converts into [`CString`].
  fn into_cstring(self) -> Result<CString>;
}

impl<T> IntoCString for T
where
  T: Into<Vec<u8>>,
{
  fn into_cstring(self) -> Result<CString> {
    CString::new(self).map_err(Error::CStringConv)
  }
}

/// Trait for converting `Option<Handle>` into handle for C API.
pub(crate) trait IntoCHandle {
  /// Converts into C handle.
  fn into_handle(self) -> u32;
}

impl IntoCHandle for Option<NonZeroU32> {
  fn into_handle(self) -> u32 {
    match self {
      Some(i) => i.into(),
      None => 0,
    }
  }
}
