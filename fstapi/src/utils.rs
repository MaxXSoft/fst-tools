use std::num::NonZeroU32;

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
