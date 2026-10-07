use crate::{Handle, VarDir, VarType};
use std::fmt;

/// Error returned by an FST API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
  /// Invalid UTF-8 string, with its decoding error when available.
  InvalidUtf8Str(Option<std::str::Utf8Error>),
  /// CStr conversion error.
  CStrConv(std::ffi::FromBytesWithNulError),
  /// CString conversion error.
  CStringConv(std::ffi::NulError),
  /// Context creation failed; libfst does not expose the underlying cause.
  ContextCreate,
  /// Invalid variable declaration, with the rejected property.
  InvalidVariableDefinition(VariableDefinitionError),
  /// The supplied handle is outside this writer's variable table.
  InvalidHandle(Handle),
  /// The alias target has incompatible storage width or real-value encoding.
  IncompatibleAlias(Handle),
  /// Handle, expected byte length, and supplied byte length.
  ValueLengthMismatch(Handle, usize, usize),
  /// Handle, expected value kind, and supplied value kind.
  ValueKindMismatch(Handle, ValueKind, ValueKind),
  /// Previous timestamp and rejected earlier timestamp.
  TimeWentBackwards(u64, u64),
  /// Limit kind, requested size in bytes, and maximum size in bytes.
  LimitExceeded(LimitKind, u64, u64),
  /// Expected physical handle number and the handle found in the hierarchy.
  InvalidHierarchy(u64, u32),
  /// Raw callback handle and the violated callback contract.
  InvalidCallbackData(u32, CallbackError),
  /// Failed libfst operation whose underlying cause is unavailable.
  BackendFailure(BackendOperation),
  /// libfst reports that its configured output dump-size limit was reached.
  DumpSizeLimitReached,
  /// libfst reports a seek failure, without the original OS error code.
  SeekFailed,
  /// VCD export failed during input processing or output I/O.
  VcdExportFailed,
}

/// Invalid property of a variable declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableDefinitionError {
  /// Unsupported variable type code.
  InvalidType(VarType),
  /// Unsupported variable direction code.
  InvalidDirection(VarDir),
  /// EVCD encoded byte width, which must equal three times a positive bit width plus two.
  InvalidPortWidth(u32),
}

/// Value representation expected by an emitter or delivered by a callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
  /// A fixed-width value, including a real value.
  Fixed,
  /// A variable-length value.
  Variable,
}

/// Storage or representation limit enforced before calling libfst or forming a slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
  /// Storage width of one variable.
  VariableWidth,
  /// Sum of the storage widths of all physical variables.
  FrameBytes,
  /// Payload of one value change, excluding section and record overhead.
  ValueRecordBytes,
  /// Buffered value-change section, including section and record overhead.
  SectionValueBytes,
  /// Callback value length representable by a Rust slice.
  CallbackBytes,
  /// EVCD encoded width representable by libfst's unsigned 32-bit length.
  EvcdEncodedWidth,
}

/// Invalid data received from libfst's value-change callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackError {
  /// The raw handle is zero or outside the cached signal table.
  InvalidHandle,
  /// Expected value kind and the kind supplied by the callback.
  ValueKindMismatch(ValueKind, ValueKind),
  /// A callback that requires data supplied a null value pointer.
  NullValuePointer,
}

/// libfst operation that returned failure without a detailed error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendOperation {
  /// Creating a variable returned no valid handle.
  CreateVariable,
  /// Traversing value-change blocks returned failure.
  ReadBlocks,
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::InvalidUtf8Str(None) => write!(f, "invalid UTF-8 string"),
      Self::InvalidUtf8Str(Some(e)) => write!(f, "{e}"),
      Self::CStrConv(e) => write!(f, "CStr conversion error, {e}"),
      Self::CStringConv(e) => write!(f, "CString conversion error, {e}"),
      Self::ContextCreate => write!(f, "context creation error"),
      Self::InvalidVariableDefinition(reason) => write!(f, "invalid variable definition: {reason}"),
      Self::InvalidHandle(handle) => write!(f, "unknown writer handle {handle}"),
      Self::IncompatibleAlias(handle) => {
        write!(f, "incompatible storage for alias of handle {handle}")
      }
      Self::ValueLengthMismatch(handle, expected, actual) => write!(
        f,
        "value length mismatch for handle {handle}: expected {expected} bytes, got {actual}"
      ),
      Self::ValueKindMismatch(handle, expected, actual) => write!(
        f,
        "value kind mismatch for handle {handle}: expected {expected}, got {actual}"
      ),
      Self::TimeWentBackwards(previous, requested) => write!(
        f,
        "time went backwards: previous timestamp {previous}, requested {requested}"
      ),
      Self::LimitExceeded(kind, requested, limit) => write!(
        f,
        "{kind} limit exceeded: requested {requested} bytes, limit {limit} bytes"
      ),
      Self::InvalidHierarchy(expected, actual) => write!(
        f,
        "invalid hierarchy: expected physical handle {expected}, got {actual}"
      ),
      Self::InvalidCallbackData(handle, reason) => {
        write!(f, "invalid callback data for handle {handle}: {reason}")
      }
      Self::BackendFailure(operation) => write!(f, "libfst failed to {operation}"),
      Self::DumpSizeLimitReached => write!(f, "libfst output dump-size limit reached"),
      Self::SeekFailed => write!(f, "libfst seek failed"),
      Self::VcdExportFailed => write!(f, "VCD export failed"),
    }
  }
}

impl std::error::Error for Error {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::InvalidUtf8Str(Some(error)) => Some(error),
      Self::CStrConv(error) => Some(error),
      Self::CStringConv(error) => Some(error),
      Self::InvalidVariableDefinition(error) => Some(error),
      Self::InvalidCallbackData(_, error) => Some(error),
      _ => None,
    }
  }
}

impl std::error::Error for VariableDefinitionError {}

impl std::error::Error for CallbackError {}

impl fmt::Display for VariableDefinitionError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::InvalidType(ty) => write!(f, "unsupported type {ty}"),
      Self::InvalidDirection(dir) => write!(f, "unsupported direction {dir}"),
      Self::InvalidPortWidth(width) => write!(f, "invalid EVCD encoded width {width}"),
    }
  }
}

impl fmt::Display for ValueKind {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    f.write_str(match self {
      Self::Fixed => "fixed-width value",
      Self::Variable => "variable-length value",
    })
  }
}

impl fmt::Display for LimitKind {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    f.write_str(match self {
      Self::VariableWidth => "variable width",
      Self::FrameBytes => "frame size",
      Self::ValueRecordBytes => "value record payload",
      Self::SectionValueBytes => "value-change section size",
      Self::CallbackBytes => "callback slice length",
      Self::EvcdEncodedWidth => "EVCD encoded width",
    })
  }
}

impl fmt::Display for CallbackError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::InvalidHandle => f.write_str("unknown handle"),
      Self::ValueKindMismatch(expected, actual) => write!(f, "expected {expected}, got {actual}"),
      Self::NullValuePointer => f.write_str("null value pointer"),
    }
  }
}

impl fmt::Display for BackendOperation {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    f.write_str(match self {
      Self::CreateVariable => "create variable",
      Self::ReadBlocks => "read value-change blocks",
    })
  }
}
