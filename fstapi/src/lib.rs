//! Rust wrapper of APIs for manipulating Fast Signal Trace (FST) format waveforms.
//!
//! FST is an open source file format for storing digital waveforms from HDL simulations. It was created by the author of [GTKWave](https://github.com/gtkwave/gtkwave) in 2014, as an alternate to the [VCD](https://en.wikipedia.org/wiki/Value_change_dump) (Value Change Dump) format.
//!
//! For more details, please see:
//!
//! * The [source code](https://github.com/gtkwave/gtkwave/tree/e1c01753bc5db9f7b42e41b9bde651a375ec5eba/gtkwave4/src/helpers/fst) of GTKWave.
//! * The [documentation](https://gtkwave.sourceforge.net/gtkwave.pdf) of GTKWave.
//! * An [unofficial specification](https://blog.timhutt.co.uk/fst_spec/) for FST format.
//!
//! # Examples
//!
//! Create an FST waveform:
//!
//! ```no_run
//! use fstapi::{Writer, var_type, var_dir};
//!
//! # fn main() -> fstapi::Result<()> {
//! // Create the waveform.
//! let mut writer = Writer::create("hello.fst", true)?
//!   .comment("FST waveform example")?
//!   .timescale_from_str("1ns")?;
//!
//! // Create a variable.
//! let var = writer.create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "var", None)?;
//!
//! // Emit value change data and time change data.
//! writer.emit_value_change(var, b"10001000")?;
//! writer.emit_time_change(10)?;
//! writer.emit_value_change(var, b"10011100")?;
//! writer.emit_time_change(42)?;
//! writer.emit_value_change(var, b"00111001")?;
//! writer.emit_time_change(100)?;
//! # Ok(())
//! # }
//! ```
//!
//! Print all variables of an FST waveform:
//!
//! ```no_run
//! # fn main() -> fstapi::Result<()> {
//! let mut reader = fstapi::Reader::open("hello.fst")?;
//! for var in reader.vars() {
//!   let (name, _) = var?;
//!   println!("{name}");
//! }
//! # Ok(())
//! # }
//! ```
//! 
//! # More Examples
//! 
//! See the GitHub repository: [fst-tools](https://github.com/MaxXSoft/fst-tools),
//! which contains 3 command line tools with this library
//! for manipulating FST waveforms.

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
  /// CStr conversion error.
  CStrConv(std::ffi::FromBytesWithNulError),
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
      Self::CStrConv(e) => write!(f, "CStr conversion error, {e}"),
      Self::CStringConv(e) => write!(f, "CString conversion error, {e}"),
      Self::ContextCreate => write!(f, "context creation error"),
      Self::InvalidOperation => write!(f, "invalid operation"),
    }
  }
}

/// Result with error type [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
