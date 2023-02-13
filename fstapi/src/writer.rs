use crate::consts::{AttrType, FileType, PackType, ScopeType, VarDir, VarType};
use crate::types::Handle;
use crate::utils::*;
use crate::{capi, Error, Result};
use std::os::raw;
use std::path::Path;

/// FST waveform writer.
#[derive(Debug)]
pub struct Writer {
  /// Non-null context pointer.
  ctx: *mut raw::c_void,
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
      Ok(Self { ctx })
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

  /// Sets file type.
  pub fn file_type(self, ty: FileType) -> Self {
    unsafe { capi::fstWriterSetFileType(self.ctx, ty) };
    self
  }

  /// Sets pack type.
  pub fn pack_type(self, ty: PackType) -> Self {
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

  /// Sets attribute begin.
  pub fn set_attr_begin(&mut self, ty: AttrType, sub_ty: i32, name: &str, arg: u64) -> Result<()> {
    let name = name.into_cstring()?;
    unsafe { capi::fstWriterSetAttrBegin(self.ctx, ty, sub_ty, name.as_ptr(), arg) };
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
  pub fn create_var(
    &mut self,
    ty: VarType,
    dir: VarDir,
    len: u32,
    name: &str,
    alias: Option<Handle>,
  ) -> Result<Handle> {
    let name = name.into_cstring()?;
    Handle::new(unsafe {
      capi::fstWriterCreateVar(self.ctx, ty, dir, len, name.as_ptr(), alias.into_handle())
    })
    .ok_or(Error::NullHandle)
  }

  /// Emits value change for the given handle.
  pub fn emit_value_change(&mut self, handle: Handle, value: &[u8]) {
    unsafe {
      capi::fstWriterEmitValueChange(
        self.ctx,
        handle.into(),
        value.as_ptr() as *const raw::c_void,
      )
    }
  }

  /// Emits vairable-length value change for the given handle.
  pub fn emit_var_len_value_change(&mut self, handle: Handle, value: &[u8]) {
    unsafe {
      capi::fstWriterEmitVariableLengthValueChange(
        self.ctx,
        handle.into(),
        value.as_ptr() as *const raw::c_void,
        value.len() as u32,
      )
    }
  }

  /// Emits time change.
  pub fn emit_time_change(&mut self, time: u64) {
    unsafe { capi::fstWriterEmitTimeChange(self.ctx, time) }
  }

  /// Flushes the content of the current writer to file.
  pub fn flush(&mut self) {
    unsafe { capi::fstWriterFlushContext(self.ctx) }
  }
}

impl Drop for Writer {
  fn drop(&mut self) {
    unsafe { capi::fstWriterClose(self.ctx) }
  }
}
