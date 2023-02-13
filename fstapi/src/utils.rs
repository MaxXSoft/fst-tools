use crate::{Error, Result};
use std::ffi::CString;
use std::num::NonZeroU32;

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
