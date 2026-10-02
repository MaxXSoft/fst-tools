use crate::consts::{AttrType, FileType, ScopeType, VarDir, VarType};
use crate::types::Handle;
use crate::utils::*;
use crate::{Error, Result, capi};
use std::any::Any;
use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::os::raw;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::{ptr, slice};

/// FST waveform reader.
#[derive(Debug)]
pub struct Reader {
  /// Non-null context pointer.
  ctx: *mut capi::fstReaderContext,
  signal_values: Option<Vec<SignalValueKind>>,
  native_doubles: bool,
}

impl Reader {
  /// Opens an FST waveform from the given path.
  pub fn open<P>(path: P) -> Result<Self>
  where
    P: AsRef<Path>,
  {
    let path = path.to_str()?.into_cstring()?;
    let ctx = unsafe { capi::fstReaderOpen(path.as_ptr()) };
    if ctx.is_null() {
      Err(Error::ContextCreate)
    } else {
      Ok(Self {
        ctx,
        signal_values: None,
        native_doubles: false,
      })
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

  /// Returns timescale as string.
  ///
  /// Returns [`None`] if the timescale is not valid.
  pub fn timescale_str(&self) -> Option<&'static str> {
    match self.timescale() + 21 {
      t @ 0..=23 => Some(
        [
          "1zs", "10zs", "100zs", "1as", "10as", "100as", "1fs", "10fs", "100fs", "1ps", "10ps",
          "100ps", "1ns", "10ns", "100ns", "1us", "10us", "100us", "1ms", "10ms", "100ms", "1s",
          "10s", "100s",
        ][t as usize],
      ),
      _ => None,
    }
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

  /// Sets whether to use native doubles in callback when iterating over blocks.
  pub fn set_native_doubles_on_callback(&mut self, enable: bool) {
    unsafe { capi::fstReaderIterBlocksSetNativeDoublesOnCallback(self.ctx, enable as i32) };
    self.native_doubles = enable;
  }

  /// Returns an iterator over the hierarchies of the waveform.
  pub fn hiers(&mut self) -> Hiers<'_> {
    unsafe { capi::fstReaderIterateHierRewind(self.ctx) };
    Hiers {
      ctx: self.ctx,
      phantom: PhantomData,
    }
  }

  /// Returns an iterator over the variables of the waveform.
  pub fn vars(&mut self) -> Vars<'_> {
    Vars {
      hiers: self.hiers(),
      scopes: Vec::new(),
    }
  }

  /// Runs the given callback on each block of the waveform.
  ///
  /// If the callback panics, further callbacks are skipped while libfst finishes
  /// the traversal, then the panic resumes after its C stack has returned.
  ///
  /// The callback will be called when value changes, and is defined as:
  ///
  /// ```
  /// fn callback(time: u64, handle: fstapi::Handle, value: &[u8], var_len: bool) {
  ///   // ...
  /// }
  /// ```
  pub fn for_each_block<F>(&mut self, mut callback: F) -> Result<()>
  where
    F: FnMut(u64, Handle, &[u8], bool),
  {
    self.cache_signal_values()?;

    extern "C" fn c_callback<F>(
      data: *mut raw::c_void,
      time: u64,
      handle: capi::fstHandle,
      value: *const raw::c_uchar,
    ) where
      F: FnMut(u64, Handle, &[u8], bool),
    {
      let state = unsafe { &mut *data.cast::<BlockCallback<'_, F>>() };
      state.invoke(time, handle, value, None);
    }

    extern "C" fn c_callback_var_len<F>(
      data: *mut raw::c_void,
      time: u64,
      handle: capi::fstHandle,
      value: *const raw::c_uchar,
      len: u32,
    ) where
      F: FnMut(u64, Handle, &[u8], bool),
    {
      let state = unsafe { &mut *data.cast::<BlockCallback<'_, F>>() };
      state.invoke(time, handle, value, Some(len));
    }

    let mut state = BlockCallback {
      callback: &mut callback,
      signal_values: self.signal_values.as_deref().unwrap(),
      native_doubles: self.native_doubles,
      panic: None,
      invalid_value: false,
    };
    let ret = unsafe {
      capi::fstReaderIterBlocks2(
        self.ctx,
        Some(c_callback::<F>),
        Some(c_callback_var_len::<F>),
        (&mut state as *mut BlockCallback<'_, F>).cast(),
        ptr::null_mut(),
      )
    };
    // A callback cannot unwind through C. Let libfst finish and release its
    // traversal buffers before continuing the panic on the Rust side.
    if let Some(payload) = state.panic {
      resume_unwind(payload);
    }
    if ret == 0 || state.invalid_value {
      Err(Error::InvalidOperation)
    } else {
      Ok(())
    }
  }

  /// Caches the encoding of each physical facility, without duplicating aliases.
  fn cache_signal_values(&mut self) -> Result<()> {
    if self.signal_values.is_some() {
      return Ok(());
    }
    let mut values = Vec::new();
    for hier in self.hiers() {
      let Hier::Var(var) = hier else { continue };
      if var.is_alias() {
        continue;
      }
      if u32::from(var.handle()) as usize != values.len() + 1 {
        return Err(Error::InvalidOperation);
      }
      let kind = match var.ty() {
        crate::var_type::VCD_REAL
        | crate::var_type::VCD_REAL_PARAMETER
        | crate::var_type::VCD_REALTIME
        | crate::var_type::SV_SHORTREAL => SignalValueKind::Real,
        crate::var_type::GEN_STRING => SignalValueKind::Variable,
        _ if var.length() == 0 => SignalValueKind::Variable,
        ty => {
          // IterateHier exposes the bit width of an EVCD port; callbacks carry
          // its value and two strengths, separated by two spaces.
          let len = if ty == crate::var_type::VCD_PORT {
            var
              .length()
              .checked_mul(3)
              .and_then(|len| len.checked_add(2))
          } else {
            Some(var.length())
          }
          .ok_or(Error::InvalidOperation)?;
          if len as u64 > isize::MAX as u64 {
            return Err(Error::InvalidOperation);
          }
          SignalValueKind::Fixed(len as usize)
        }
      };
      values.push(kind);
    }
    self.signal_values = Some(values);
    Ok(())
  }

  /// Dumps the content of waveform as VCD format to the given file
  /// ([Some(path)]) or the standard output ([None]).
  ///
  /// Enables the process mask for every facility. Existing time range limits
  /// remain in effect.
  pub fn dump_as_vcd<P>(&mut self, path: Option<P>) -> Result<()>
  where
    P: AsRef<Path>,
  {
    let ret = if let Some(path) = path {
      let path = path.to_str()?.into_cstring()?;
      unsafe { capi::fstToolsReaderDumpToVcdFile(self.ctx, path.as_ptr()) }
    } else {
      unsafe { capi::fstToolsReaderDumpToVcdFile(self.ctx, ptr::null()) }
    };
    match ret {
      0 => Ok(()),
      _ => Err(Error::InvalidOperation),
    }
  }
}

impl Drop for Reader {
  fn drop(&mut self) {
    unsafe { capi::fstReaderClose(self.ctx) }
  }
}

#[derive(Clone, Copy, Debug)]
enum SignalValueKind {
  Fixed(usize),
  Real,
  Variable,
}

struct BlockCallback<'a, F> {
  callback: &'a mut F,
  signal_values: &'a [SignalValueKind],
  native_doubles: bool,
  panic: Option<Box<dyn Any + Send>>,
  invalid_value: bool,
}

impl<F> BlockCallback<'_, F>
where
  F: FnMut(u64, Handle, &[u8], bool),
{
  fn invoke(&mut self, time: u64, handle: u32, value: *const u8, var_len: Option<u32>) {
    if self.panic.is_some() || self.invalid_value {
      return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
      let handle = Handle::new(handle).ok_or(Error::InvalidOperation)?;
      let kind = self
        .signal_values
        .get(u32::from(handle) as usize - 1)
        .ok_or(Error::InvalidOperation)?;
      let len = match (kind, var_len) {
        (SignalValueKind::Variable, Some(len)) => len as usize,
        (SignalValueKind::Fixed(len), None) => *len,
        (SignalValueKind::Real, None) if self.native_doubles => 8,
        (SignalValueKind::Real, None) if !value.is_null() => {
          // Only textual real callbacks require a strlen scan. In native mode
          // the double is binary and can contain NUL bytes.
          unsafe { CStr::from_ptr(value.cast()) }.to_bytes().len()
        }
        _ => return Err(Error::InvalidOperation),
      };
      if len > isize::MAX as usize || (len != 0 && value.is_null()) {
        return Err(Error::InvalidOperation);
      }
      let value = if len == 0 {
        &[]
      } else {
        unsafe { slice::from_raw_parts(value, len) }
      };
      (self.callback)(time, handle, value, var_len.is_some());
      Ok(())
    }));
    match result {
      Ok(Ok(())) => {}
      Ok(Err(_)) => self.invalid_value = true,
      Err(payload) => self.panic = Some(payload),
    }
  }
}

/// An iterator over the hierarchies of an FST waveform.
///
/// This struct is created by the [`hiers`](Reader::hiers)
/// method on [`Reader`].
#[derive(Debug)]
pub struct Hiers<'a> {
  ctx: *mut capi::fstReaderContext,
  phantom: PhantomData<&'a mut Reader>,
}

impl<'a> Iterator for Hiers<'a> {
  type Item = Hier<'a>;

  fn next(&mut self) -> Option<Self::Item> {
    unsafe { capi::fstReaderIterateHier(self.ctx).as_ref() }.map(Hier::new)
  }
}

/// Hierarchy of FST waveform.
///
/// Each item owns its data and remains valid when its iterator advances.
#[derive(Debug)]
pub enum Hier<'a> {
  /// Begin of a scope.
  Scope(Scope<'a>),
  /// End of a scope.
  Upscope,
  /// Variable.
  Var(Var<'a>),
  /// Begin of an attribute.
  AttrBegin(Attr<'a>),
  /// End of an attribute.
  AttrEnd,
}

impl Hier<'_> {
  /// Copies the record and strings before libfst reuses its hierarchy storage.
  fn new(hier: &capi::fstHier) -> Self {
    match hier.htyp as capi::fstHierType {
      capi::fstHierType_FST_HT_SCOPE => {
        let scope = unsafe { &hier.u.scope };
        Self::Scope(Scope {
          ty: scope.typ as ScopeType,
          name: unsafe { CStr::from_ptr(scope.name) }.to_owned(),
          component: unsafe { CStr::from_ptr(scope.component) }.to_owned(),
          phantom: PhantomData,
        })
      }
      capi::fstHierType_FST_HT_UPSCOPE => Self::Upscope,
      capi::fstHierType_FST_HT_VAR => {
        let var = unsafe { &hier.u.var };
        Self::Var(Var {
          ty: var.typ as VarType,
          direction: var.direction as VarDir,
          name: unsafe { CStr::from_ptr(var.name) }.to_owned(),
          length: var.length,
          handle: Handle::new(var.handle).expect("libfst returned a zero variable handle"),
          is_alias: var.is_alias() != 0,
          phantom: PhantomData,
        })
      }
      capi::fstHierType_FST_HT_ATTRBEGIN => {
        let attr = unsafe { &hier.u.attr };
        Self::AttrBegin(Attr {
          ty: attr.typ as AttrType,
          subtype: attr.subtype as u32,
          name: unsafe { CStr::from_ptr(attr.name) }.to_owned(),
          arg: attr.arg,
          arg_from_name: attr.arg_from_name,
          phantom: PhantomData,
        })
      }
      capi::fstHierType_FST_HT_ATTREND => Self::AttrEnd,
      _ => unreachable!("libfst returned an unknown hierarchy type"),
    }
  }
}

/// A scope in FST hierarchy.
#[derive(Debug)]
pub struct Scope<'a> {
  ty: ScopeType,
  name: CString,
  component: CString,
  phantom: PhantomData<&'a ()>,
}

impl Scope<'_> {
  /// Returns scope type.
  pub fn ty(&self) -> ScopeType {
    self.ty
  }

  /// Returns scope name.
  pub fn name(&self) -> Result<&str> {
    self
      .name
      .to_str()
      .map_err(|e| Error::InvalidUtf8Str(Some(e)))
  }

  /// Returns scope name as a C string, valid until this scope is dropped.
  pub fn name_raw(&self) -> *const raw::c_char {
    self.name.as_ptr()
  }

  /// Returns scope component.
  pub fn component(&self) -> Result<&str> {
    self
      .component
      .to_str()
      .map_err(|e| Error::InvalidUtf8Str(Some(e)))
  }

  /// Returns scope component as a C string, valid until this scope is dropped.
  pub fn component_raw(&self) -> *const raw::c_char {
    self.component.as_ptr()
  }
}

/// A variable in FST hierarchy.
#[derive(Debug)]
pub struct Var<'a> {
  ty: VarType,
  direction: VarDir,
  name: CString,
  length: u32,
  handle: Handle,
  is_alias: bool,
  phantom: PhantomData<&'a ()>,
}

impl Var<'_> {
  /// Returns variable type.
  pub fn ty(&self) -> VarType {
    self.ty
  }

  /// Returns variable direction.
  pub fn direction(&self) -> VarDir {
    self.direction
  }

  /// Returns variable name.
  pub fn name(&self) -> Result<&str> {
    self
      .name
      .to_str()
      .map_err(|e| Error::InvalidUtf8Str(Some(e)))
  }

  /// Returns variable name as a C string, valid until this variable is dropped.
  pub fn name_raw(&self) -> *const raw::c_char {
    self.name.as_ptr()
  }

  /// Returns variable length in bits.
  pub fn length(&self) -> u32 {
    self.length
  }

  /// Returns variable handle.
  pub fn handle(&self) -> Handle {
    self.handle
  }

  /// Returns `true` if variable is an alias.
  pub fn is_alias(&self) -> bool {
    self.is_alias
  }
}

/// An attribute in FST hierarchy.
#[derive(Debug)]
pub struct Attr<'a> {
  ty: AttrType,
  subtype: u32,
  name: CString,
  arg: u64,
  arg_from_name: u64,
  phantom: PhantomData<&'a ()>,
}

impl Attr<'_> {
  /// Returns attribute type.
  pub fn ty(&self) -> AttrType {
    self.ty
  }

  /// Returns attribute subtype.
  ///
  /// The subtype may be one of [`MiscType`](crate::consts::MiscType),
  /// [`ArrayType`](crate::consts::ArrayType),
  /// [`EnumValueType`](crate::consts::EnumValueType) or
  /// [`PackType`](crate::consts::PackType).
  pub fn subtype(&self) -> u32 {
    self.subtype
  }

  /// Returns attribute name.
  pub fn name(&self) -> Result<&str> {
    self
      .name
      .to_str()
      .map_err(|e| Error::InvalidUtf8Str(Some(e)))
  }

  /// Returns attribute name as a C string, valid until this attribute is dropped.
  pub fn name_raw(&self) -> *const raw::c_char {
    self.name.as_ptr()
  }

  /// Returns attribute argument.
  ///
  /// Argument may be number of array elements, struct members,
  /// or some other payload (possibly ignored).
  pub fn arg(&self) -> u64 {
    self.arg
  }

  /// Returns attribute argument generated by the attribute name.
  ///
  /// For when name is overloaded as a variable-length integer,
  /// i.e. `ty` is [`attr_type::MISC`](crate::consts::attr_type::MISC)
  /// and `subtype` is
  /// [`misc_type::SOURCESTEM`](crate::consts::misc_type::SOURCESTEM) or
  /// [`misc_type::SOURCEISTEM`](crate::consts::misc_type::SOURCEISTEM).
  pub fn arg_from_name(&self) -> u64 {
    self.arg_from_name
  }
}

/// An iterator over the variables of an FST waveform.
///
/// This struct is created by the [`vars`](Reader::vars)
/// method on [`Reader`].
#[derive(Debug)]
pub struct Vars<'a> {
  hiers: Hiers<'a>,
  scopes: Vec<String>,
}

impl<'a> Iterator for Vars<'a> {
  type Item = Result<(String, Var<'a>)>;

  fn next(&mut self) -> Option<Self::Item> {
    macro_rules! unwrap_or_return {
      ($e:expr) => {
        match $e {
          Ok(v) => v,
          Err(e) => return Some(Err(e)),
        }
      };
    }

    for hier in self.hiers.by_ref() {
      match hier {
        Hier::Scope(s) => {
          let name = unwrap_or_return!(s.name());
          match self.scopes.last() {
            Some(last) => self.scopes.push(format!("{last}.{name}")),
            None => self.scopes.push(name.into()),
          }
        }
        Hier::Upscope => {
          self.scopes.pop();
        }
        Hier::Var(v) => {
          let name = unwrap_or_return!(v.name());
          let name = match self.scopes.last() {
            Some(last) => format!("{last}.{name}"),
            None => name.into(),
          };
          return Some(Ok((name, v)));
        }
        _ => {}
      }
    }

    None
  }
}
