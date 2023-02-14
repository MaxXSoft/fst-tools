use crate::consts::{AttrType, FileType, ScopeType, VarDir, VarType};
use crate::types::Handle;
use crate::utils::*;
use crate::{capi, Error, Result};
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::os::raw;
use std::path::Path;
use std::{ptr, slice};

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

  /// Returns an iterator over the hierarchies of the waveform.
  pub fn hiers(&mut self) -> Hiers {
    unsafe { capi::fstReaderIterateHierRewind(self.ctx) };
    Hiers {
      ctx: self.ctx,
      phantom: PhantomData,
    }
  }

  /// Runs the given callbacks on each block of the waveform.
  ///
  /// The first callback will be called when value changes, the second callback
  /// will be called when variable-length value changes.
  ///
  /// The callback function is defined as:
  ///
  /// ```
  /// fn callback(time: u64, handle: fstapi::Handle, value: &[u8]) {
  ///   // ...
  /// }
  /// ```
  pub fn for_each_block<F, F2>(&mut self, callback: F, callback_var_len: F2) -> Result<()>
  where
    F: FnMut(u64, Handle, &[u8]),
    F2: FnMut(u64, Handle, &[u8]),
  {
    extern "C" fn c_callback<F, F2>(
      data: *mut raw::c_void,
      time: u64,
      handle: capi::fstHandle,
      value: *const raw::c_uchar,
      len: u32,
    ) where
      F: FnMut(u64, Handle, &[u8]),
      F2: FnMut(u64, Handle, &[u8]),
    {
      let data: &mut (F, F2) = unsafe { &mut *(data as *mut (F, F2)) };
      let handle = unsafe { Handle(NonZeroU32::new_unchecked(handle)) };
      let value = unsafe { slice::from_raw_parts(value, len as usize) };
      data.0(time, handle, value);
    }

    extern "C" fn c_callback_var_len<F, F2>(
      data: *mut raw::c_void,
      time: u64,
      handle: capi::fstHandle,
      value: *const raw::c_uchar,
      len: u32,
    ) where
      F: FnMut(u64, Handle, &[u8]),
      F2: FnMut(u64, Handle, &[u8]),
    {
      let data: &mut (F, F2) = unsafe { &mut *(data as *mut (F, F2)) };
      let handle = unsafe { Handle(NonZeroU32::new_unchecked(handle)) };
      let value = unsafe { slice::from_raw_parts(value, len as usize) };
      data.1(time, handle, value);
    }

    let mut data = (callback, callback_var_len);
    let ret = unsafe {
      capi::fstReaderIterBlocks2(
        self.ctx,
        Some(c_callback::<F, F2>),
        Some(c_callback_var_len::<F, F2>),
        (&mut data) as *mut _ as *mut raw::c_void,
        ptr::null_mut(),
      )
    };
    match ret {
      0 => Err(Error::InvalidOperation),
      _ => Ok(()),
    }
  }
}

impl Drop for Reader {
  fn drop(&mut self) {
    unsafe { capi::fstReaderClose(self.ctx) }
  }
}

/// An iterator over the hierarchies of a FST waveform.
///
/// This struct is created by the [`hiers`](Reader::hiers)
/// method on [`Reader`].
#[derive(Debug)]
pub struct Hiers<'a> {
  ctx: *mut raw::c_void,
  phantom: PhantomData<&'a ()>,
}

impl<'a> Iterator for Hiers<'a> {
  type Item = Hier<'a>;

  fn next(&mut self) -> Option<Self::Item> {
    unsafe { capi::fstReaderIterateHier(self.ctx).as_ref() }.map(Hier::new)
  }
}

/// Hierarchy of FST waveform.
pub enum Hier<'a> {
  Scope(Scope<'a>),
  Upscope,
  Var(Var<'a>),
  AttrBegin(Attr<'a>),
  AttrEnd,
}

impl<'a> Hier<'a> {
  /// Creates a new hierarchy.
  fn new(hier: &'a capi::fstHier) -> Self {
    match hier.htyp as u32 {
      capi::fstHierType_FST_HT_SCOPE => Self::Scope(Scope(unsafe { &hier.u.scope })),
      capi::fstHierType_FST_HT_UPSCOPE => Self::Upscope,
      capi::fstHierType_FST_HT_VAR => Self::Var(Var(unsafe { &hier.u.var })),
      capi::fstHierType_FST_HT_ATTRBEGIN => Self::AttrBegin(Attr(unsafe { &hier.u.attr })),
      capi::fstHierType_FST_HT_ATTREND => Self::AttrEnd,
      _ => unreachable!(),
    }
  }
}

/// A scope in FST hierarchy.
pub struct Scope<'a>(&'a capi::fstHier__bindgen_ty_1_fstHierScope);

impl<'a> Scope<'a> {
  /// Returns scope type.
  pub fn ty(&self) -> ScopeType {
    self.0.typ as ScopeType
  }

  /// Returns scope name.
  pub fn name(&self) -> Result<&str> {
    unsafe { (self.0.name, self.0.name_length + 1).to_str() }
  }

  /// Returns scope name as raw C string.
  pub fn name_raw(&self) -> *const raw::c_char {
    self.0.name
  }

  /// Returns scope component.
  pub fn component(&self) -> Result<&str> {
    unsafe { (self.0.component, self.0.component_length + 1).to_str() }
  }

  /// Returns scope component as raw C string.
  pub fn component_raw(&self) -> *const raw::c_char {
    self.0.component
  }
}

/// A variable in FST hierarchy.
pub struct Var<'a>(&'a capi::fstHier__bindgen_ty_1_fstHierVar);

impl<'a> Var<'a> {
  /// Returns variable type.
  pub fn ty(&self) -> VarType {
    self.0.typ as VarType
  }

  /// Returns variable direction.
  pub fn direction(&self) -> VarDir {
    self.0.direction as VarDir
  }

  /// Returns variable name.
  pub fn name(&self) -> Result<&str> {
    unsafe { (self.0.name, self.0.name_length + 1).to_str() }
  }

  /// Returns variable name as raw C string.
  pub fn name_raw(&self) -> *const raw::c_char {
    self.0.name
  }

  /// Returns variable length in bits.
  pub fn length(&self) -> u32 {
    self.0.length
  }

  /// Returns variable handle.
  pub fn handle(&self) -> Handle {
    unsafe { Handle(NonZeroU32::new_unchecked(self.0.handle)) }
  }

  /// Returns `true` if variable is an alias.
  pub fn is_alias(&self) -> bool {
    self.0.is_alias() != 0
  }
}

/// An attribute in FST hierarchy.
pub struct Attr<'a>(&'a capi::fstHier__bindgen_ty_1_fstHierAttr);

impl<'a> Attr<'a> {
  /// Returns attribute type.
  pub fn ty(&self) -> AttrType {
    self.0.typ as AttrType
  }

  /// Returns attribute subtype.
  ///
  /// The subtype may be one of [`MiscType`](crate::consts::MiscType),
  /// [`ArrayType`](crate::consts::ArrayType),
  /// [`EnumValueType`](crate::consts::EnumValueType) or
  /// [`PackType`](crate::consts::PackType).
  pub fn subtype(&self) -> u8 {
    self.0.subtype
  }

  /// Returns attribute name.
  pub fn name(&self) -> Result<&str> {
    unsafe { (self.0.name, self.0.name_length + 1).to_str() }
  }

  /// Returns attribute name as raw C string.
  pub fn name_raw(&self) -> *const raw::c_char {
    self.0.name
  }

  /// Returns attribute argument.
  ///
  /// Argument may be number of array elements, struct members,
  /// or some other payload (possibly ignored).
  pub fn arg(&self) -> u64 {
    self.0.arg
  }

  /// Returns attribute argument generated by the attribute name.
  ///
  /// For when name is overloaded as a variable-length integer,
  /// i.e. `ty` is [`attr_type::MISC`](crate::consts::attr_type::MISC)
  /// and `subtype` is
  /// [`misc_type::SOURCESTEM`](crate::consts::misc_type::SOURCESTEM) or
  /// [`misc_type::SOURCEISTEM`](crate::consts::misc_type::SOURCEISTEM).
  pub fn arg_from_name(&self) -> u64 {
    self.0.arg_from_name
  }
}
