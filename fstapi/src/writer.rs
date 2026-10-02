use crate::consts::{AttrType, FileType, ScopeType, VarDir, VarType, WriterPackType};
use crate::types::Handle;
use crate::utils::*;
use crate::{Error, Result, capi, var_dir, var_type};
use std::os::raw;
use std::path::Path;

/// FST waveform writer.
#[derive(Debug)]
pub struct Writer {
  /// Non-null context pointer.
  ctx: *mut capi::fstWriterContext,
  /// Storage belongs to unique handles; aliases reuse an existing entry.
  variables: Vec<Variable>,
  value_bytes: u32,
  last_time: Option<u64>,
}

impl Writer {
  /// Creates a new [`Writer`], writes the output waveform to the given path.
  pub fn create<P>(path: P, use_compressed_hier: bool) -> Result<Self>
  where
    P: AsRef<Path>,
  {
    let path = path.to_str()?.into_cstring()?;
    let ctx = unsafe { capi::fstWriterCreate(path.as_ptr(), use_compressed_hier as raw::c_int) };
    if ctx.is_null() {
      Err(Error::ContextCreate)
    } else {
      Ok(Self {
        ctx,
        variables: Vec::new(),
        value_bytes: 0,
        last_time: None,
      })
    }
  }

  /// Sets comment.
  pub fn comment(self, comment: &str) -> Result<Self> {
    let comment = comment.into_cstring()?;
    unsafe { capi::fstWriterSetComment(self.ctx, comment.as_ptr()) };
    Ok(self)
  }

  /// Sets date.
  pub fn date(self, date: &str) -> Result<Self> {
    let date = date.into_cstring()?;
    unsafe { capi::fstWriterSetDate(self.ctx, date.as_ptr()) };
    Ok(self)
  }

  /// Sets date from the given raw C string.
  ///
  /// # Safety
  ///
  /// A non-null `date` must point to a valid nul-terminated string.
  /// A null pointer leaves the date unchanged.
  pub unsafe fn date_raw(self, date: *const raw::c_char) -> Self {
    if !date.is_null() {
      unsafe { capi::fstWriterSetDate(self.ctx, date) };
    }
    self
  }

  /// Sets file type.
  pub fn file_type(self, ty: FileType) -> Self {
    unsafe { capi::fstWriterSetFileType(self.ctx, ty) };
    self
  }

  /// Sets pack type.
  pub fn pack_type(self, ty: WriterPackType) -> Self {
    unsafe { capi::fstWriterSetPackType(self.ctx, ty) };
    self
  }

  /// Sets parallel mode.
  pub fn parallel_mode(self, enable: bool) -> Self {
    unsafe { capi::fstWriterSetParallelMode(self.ctx, enable as raw::c_int) };
    self
  }

  /// Sets repack on close.
  pub fn repack_on_close(self, enable: bool) -> Self {
    unsafe { capi::fstWriterSetRepackOnClose(self.ctx, enable as raw::c_int) };
    self
  }

  /// Sets timescale.
  pub fn timescale(self, timescale: i32) -> Self {
    unsafe { capi::fstWriterSetTimescale(self.ctx, timescale) };
    self
  }

  /// Sets timescale from the given string.
  pub fn timescale_from_str(self, timescale: &str) -> Result<Self> {
    let timescale = timescale.into_cstring()?;
    unsafe { capi::fstWriterSetTimescaleFromString(self.ctx, timescale.as_ptr()) };
    Ok(self)
  }

  /// Sets timezero.
  pub fn timezero(self, timezero: i64) -> Self {
    unsafe { capi::fstWriterSetTimezero(self.ctx, timezero) };
    self
  }

  /// Sets version.
  pub fn version(self, version: &str) -> Result<Self> {
    let version = version.into_cstring()?;
    unsafe { capi::fstWriterSetVersion(self.ctx, version.as_ptr()) };
    Ok(self)
  }

  /// Sets version from the given raw C string.
  ///
  /// # Safety
  ///
  /// The memory pointed to by `version` must contain a valid nul terminator
  /// at the end of the string.
  pub unsafe fn version_raw(self, version: *const raw::c_char) -> Self {
    unsafe { capi::fstWriterSetVersion(self.ctx, version) };
    self
  }

  /// Sets attribute begin.
  pub fn set_attr_begin(&mut self, ty: AttrType, sub_ty: u32, name: &str, arg: u64) -> Result<()> {
    let name = name.into_cstring()?;
    unsafe { capi::fstWriterSetAttrBegin(self.ctx, ty, sub_ty as _, name.as_ptr(), arg) };
    Ok(())
  }

  /// Sets attribute end.
  pub fn set_attr_end(&mut self) {
    unsafe { capi::fstWriterSetAttrEnd(self.ctx) }
  }

  /// Sets scope.
  pub fn set_scope(&mut self, ty: ScopeType, name: &str, component: &str) -> Result<()> {
    let name = name.into_cstring()?;
    let component = component.into_cstring()?;
    unsafe { capi::fstWriterSetScope(self.ctx, ty, name.as_ptr(), component.as_ptr()) };
    Ok(())
  }

  /// Sets upscope.
  pub fn set_upscope(&mut self) {
    unsafe { capi::fstWriterSetUpscope(self.ctx) }
  }

  /// Creates a new variable.
  ///
  /// Real types store eight-byte native doubles and strings use variable-length
  /// values. EVCD ports take their encoded length (three times the signal width
  /// plus two spaces). An alias must refer to a compatible existing handle.
  pub fn create_var(
    &mut self,
    ty: VarType,
    dir: VarDir,
    len: u32,
    name: &str,
    alias: Option<Handle>,
  ) -> Result<Handle> {
    let name = name.into_cstring()?;
    if !(var_type::MIN..=var_type::MAX).contains(&ty)
      || !(var_dir::MIN..=var_dir::MAX).contains(&dir)
      || (ty == var_type::VCD_PORT && (len < 5 || !(len - 2).is_multiple_of(3)))
    {
      return Err(Error::InvalidOperation);
    }
    let variable = Variable::new(ty, len);
    // The C writer uses a signed int for widths and uint32_t frame offsets.
    if variable.width > i32::MAX as u32 {
      return Err(Error::InvalidOperation);
    }
    let next_value_bytes = if let Some(alias) = alias {
      if self.variable(alias)? != variable {
        return Err(Error::InvalidOperation);
      }
      self.value_bytes
    } else {
      self
        .value_bytes
        .checked_add(variable.width)
        .ok_or(Error::InvalidOperation)?
    };
    let handle = Handle::new(unsafe {
      capi::fstWriterCreateVar(self.ctx, ty, dir, len, name.as_ptr(), alias.into_handle())
    })
    .ok_or(Error::InvalidOperation)?;
    if alias.is_none() {
      self.variables.push(variable);
      self.value_bytes = next_value_bytes;
    }
    Ok(handle)
  }

  /// Emits value change for the given handle.
  ///
  /// The byte length must exactly match the variable's storage width. Invalid
  /// input is rejected before calling libfst, without changing the waveform.
  /// Before the first explicit time change, the value is written at time zero.
  pub fn emit_value_change(&mut self, handle: Handle, value: &[u8]) -> Result<()> {
    let variable = self.variable(handle)?;
    if variable.width == 0 || value.len() != variable.width as usize {
      return Err(Error::InvalidOperation);
    }
    self.check_status()?;
    // Use a normal time-zero change instead of libfst's initial frame alone.
    // Otherwise an entirely static waveform can lose its value section when
    // the first explicit time change is followed directly by close.
    if self.last_time.is_none() {
      self.emit_time_change(0)?;
    }
    unsafe {
      capi::fstWriterEmitValueChange(self.ctx, handle.into(), value.as_ptr().cast());
    }
    self.check_status()
  }

  /// Emits variable-length value change for the given handle.
  ///
  /// Before the first explicit time change, the value is written at time zero.
  pub fn emit_var_len_value_change(&mut self, handle: Handle, value: &[u8]) -> Result<()> {
    if self.variable(handle)?.width != 0 {
      return Err(Error::InvalidOperation);
    }
    let len = u32::try_from(value.len()).map_err(|_| Error::InvalidOperation)?;
    self.check_status()?;
    // libfst has no initial frame for variable-length signals: initialize the
    // time chain first, or the first time change would discard this value.
    if self.last_time.is_none() {
      self.emit_time_change(0)?;
    }
    unsafe {
      capi::fstWriterEmitVariableLengthValueChange(
        self.ctx,
        handle.into(),
        value.as_ptr().cast(),
        len,
      );
    }
    self.check_status()
  }

  /// Emits time change.
  ///
  /// Times must be nondecreasing; equal timestamps are allowed. Errors from
  /// libfst's public status flags are reported, but its fatal internal failures
  /// and errors during [`Drop`] cannot be converted to this result.
  pub fn emit_time_change(&mut self, time: u64) -> Result<()> {
    if self.last_time.is_some_and(|last| time < last) {
      return Err(Error::InvalidOperation);
    }
    self.check_status()?;
    unsafe { capi::fstWriterEmitTimeChange(self.ctx, time) };
    self.last_time = Some(time);
    self.check_status()
  }

  /// Flushes the content of the current writer to file.
  pub fn flush(&mut self) {
    unsafe { capi::fstWriterFlushContext(self.ctx) }
  }

  /// Returns storage metadata for a handle, rejecting handles outside the table.
  fn variable(&self, handle: Handle) -> Result<Variable> {
    self
      .variables
      .get(u32::from(handle) as usize - 1)
      .copied()
      .ok_or(Error::InvalidOperation)
  }

  /// Reports a reached dump-size limit or seek failure from libfst's status flags.
  fn check_status(&self) -> Result<()> {
    if unsafe {
      capi::fstWriterGetDumpSizeLimitReached(self.ctx) != 0
        || capi::fstWriterGetFseekFailed(self.ctx) != 0
    } {
      Err(Error::InvalidOperation)
    } else {
      Ok(())
    }
  }
}

impl Drop for Writer {
  fn drop(&mut self) {
    unsafe { capi::fstWriterClose(self.ctx) }
  }
}

/// Normalized storage metadata used to validate value lengths and aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Variable {
  /// Storage width in bytes, or zero for variable-length values.
  width: u32,
  /// Whether values use the native-double representation.
  real: bool,
}

impl Variable {
  /// Normalizes real values to eight bytes and generic strings to variable length.
  fn new(ty: VarType, width: u32) -> Self {
    let real = matches!(
      ty,
      var_type::VCD_REAL
        | var_type::VCD_REAL_PARAMETER
        | var_type::VCD_REALTIME
        | var_type::SV_SHORTREAL
    );
    let width = if real {
      8
    } else if ty == var_type::GEN_STRING {
      0
    } else {
      width
    };
    Self { width, real }
  }
}
