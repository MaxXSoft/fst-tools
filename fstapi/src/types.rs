use crate::utils::IntoCHandle;
use std::num::NonZeroU32;

/// Handle type, which is actually a 32-bit non-zero unsigned integer.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(pub(crate) NonZeroU32);

impl Handle {
  /// Creates a new handle.
  pub(crate) fn new(n: u32) -> Option<Self> {
    NonZeroU32::new(n).map(Self)
  }
}

impl From<Handle> for u32 {
  /// Creates a non-zero unsigned integer from the handle.
  fn from(value: Handle) -> Self {
    value.0.into()
  }
}

impl IntoCHandle for Option<Handle> {
  fn into_handle(self) -> u32 {
    match self {
      Some(i) => i.into(),
      None => 0,
    }
  }
}
