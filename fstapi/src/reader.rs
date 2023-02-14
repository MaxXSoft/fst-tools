use crate::consts::FileType;
use crate::types::Handle;
use crate::utils::*;
use crate::{capi, Error, Result};
use std::os::raw;
use std::path::Path;

/// FST waveform reader.
#[derive(Debug)]
pub struct Reader {
  /// Non-null context pointer.
  ctx: *mut raw::c_void,
}

impl Reader {
  /// Opens a FST waveform from the given path.
  pub fn open<P>(path: P) -> Result<Self>
  where
    P: AsRef<Path>,
  {
    let path = path.to_str()?.into_cstring()?;
    let ctx = unsafe { capi::fstReaderOpen(path.as_ptr()) };
    if ctx.is_null() {
      Err(Error::ContextCreate)
    } else {
      Ok(Self { ctx })
    }
  }

  /// Returns date.
  pub fn date(&self) -> Result<&str> {
    unsafe { capi::fstReaderGetDateString(self.ctx).to_str() }
  }

  /// Returns date as raw C string.
  pub fn date_raw(&self) -> *const raw::c_char {
    unsafe { capi::fstReaderGetDateString(self.ctx) }
  }

  /// Returns version.
  pub fn version(&self) -> Result<&str> {
    unsafe { capi::fstReaderGetVersionString(self.ctx).to_str() }
  }

  /// Returns version as raw C string.
  pub fn version_raw(&self) -> *const raw::c_char {
    unsafe { capi::fstReaderGetVersionString(self.ctx) }
  }

  /// Returns start time.
  pub fn start_time(&self) -> u64 {
    unsafe { capi::fstReaderGetStartTime(self.ctx) }
  }

  /// Returns end time.
  pub fn end_time(&self) -> u64 {
    unsafe { capi::fstReaderGetEndTime(self.ctx) }
  }

  /// Returns file type.
  pub fn file_type(&self) -> FileType {
    unsafe { capi::fstReaderGetFileType(self.ctx) as FileType }
  }

  /// Returns alias count.
  pub fn alias_count(&self) -> u64 {
    unsafe { capi::fstReaderGetAliasCount(self.ctx) }
  }

  /// Returns scope count.
  pub fn scope_count(&self) -> u64 {
    unsafe { capi::fstReaderGetScopeCount(self.ctx) }
  }

  /// Returns variable count.
  pub fn var_count(&self) -> u64 {
    unsafe { capi::fstReaderGetVarCount(self.ctx) }
  }

  /// Returns timescale.
  pub fn timescale(&self) -> i32 {
    unsafe { capi::fstReaderGetTimescale(self.ctx) as i32 }
  }

  /// Returns timezero.
  pub fn timezero(&self) -> i64 {
    unsafe { capi::fstReaderGetTimezero(self.ctx) }
  }

  /// Returns process mask for the facility of the given handle.
  pub fn mask(&self, handle: Handle) -> bool {
    unsafe { capi::fstReaderGetFacProcessMask(self.ctx, handle.into()) != 0 }
  }

  /// Clears process mask for the facility of the given handle.
  pub fn clear_mask(&mut self, handle: Handle) {
    unsafe { capi::fstReaderClrFacProcessMask(self.ctx, handle.into()) }
  }

  /// Clears process mask for all facilities.
  pub fn clear_mask_all(&mut self) {
    unsafe { capi::fstReaderClrFacProcessMaskAll(self.ctx) }
  }

  /// Sets process mask for the facility of the given handle.
  pub fn set_mask(&mut self, handle: Handle) {
    unsafe { capi::fstReaderSetFacProcessMask(self.ctx, handle.into()) }
  }

  /// Sets process mask for all facilities.
  pub fn set_mask_all(&mut self) {
    unsafe { capi::fstReaderSetFacProcessMaskAll(self.ctx) }
  }

  /// Sets time range limit.
  pub fn set_time_range_limit(&mut self, start_time: u64, end_time: u64) {
    unsafe { capi::fstReaderSetLimitTimeRange(self.ctx, start_time, end_time) }
  }

  /// Resets time range limit.
  pub fn reset_time_range_limit(&mut self) {
    unsafe { capi::fstReaderSetUnlimitedTimeRange(self.ctx) }
  }
}

impl Drop for Reader {
  fn drop(&mut self) {
    unsafe { capi::fstReaderClose(self.ctx) }
  }
}
