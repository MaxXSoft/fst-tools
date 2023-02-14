use crate::utils::IntoCHandle;
use std::num::NonZeroU32;

/// Defines a new handle type.
macro_rules! def_handle_type {
  ($name:ident) => {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct $name(pub(crate) NonZeroU32);

    impl $name {
      /// Creates a new handle.
      pub(crate) fn new(n: u32) -> Option<Self> {
        NonZeroU32::new(n).map(Self)
      }
    }

    impl From<$name> for u32 {
      fn from(value: $name) -> Self {
        value.0.into()
      }
    }

    impl IntoCHandle for Option<$name> {
      fn into_handle(self) -> u32 {
        match self {
          Some(i) => i.into(),
          None => 0,
        }
      }
    }
  };
}

def_handle_type!(Handle);
