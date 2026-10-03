use crate::consts::{AttrType, FileType, ScopeType, VarDir, VarType, WriterPackType};
use crate::types::Handle;
use crate::utils::*;
use crate::{
  BackendOperation, Error, LimitKind, Result, ValueKind, VariableDefinitionError, capi, var_dir,
  var_type,
};
use std::ffi::CStr;
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
  /// Conservative encoded size of the current value-change section.
  buffered_value_bytes: u32,
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
        buffered_value_bytes: 1,
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
  ///
  /// Subtype constants use target-dependent C enum types. Cast them with `as _`
  /// when passing them to the `u32` subtype parameter.
  pub fn set_attr_begin(&mut self, ty: AttrType, sub_ty: u32, name: &str, arg: u64) -> Result<()> {
    let name = name.into_cstring()?;
    self.set_attr_begin_raw(ty, sub_ty, &name, arg)
  }

  /// Sets attribute begin, preserving names containing non-UTF-8 bytes.
  ///
  /// Some FST attributes encode binary integers in their nul-terminated names.
  /// Convert subtype constants with `as _`, as for [`Self::set_attr_begin`].
  pub fn set_attr_begin_raw(
    &mut self,
    ty: AttrType,
    sub_ty: u32,
    name: &CStr,
    arg: u64,
  ) -> Result<()> {
    self.check_status()?;
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
    if !(var_type::MIN..=var_type::MAX).contains(&ty) {
      return Err(Error::InvalidVariableDefinition(
        VariableDefinitionError::InvalidType(ty),
      ));
    }
    if !(var_dir::MIN..=var_dir::MAX).contains(&dir) {
      return Err(Error::InvalidVariableDefinition(
        VariableDefinitionError::InvalidDirection(dir),
      ));
    }
    if ty == var_type::VCD_PORT && (len < 5 || !(len - 2).is_multiple_of(3)) {
      return Err(Error::InvalidVariableDefinition(
        VariableDefinitionError::InvalidPortWidth(len),
      ));
    }
    let variable = Variable::new(ty, len);
    // The C writer uses a signed int for widths and uint32_t frame offsets.
    if variable.width > i32::MAX as u32 {
      return Err(Error::LimitExceeded(
        LimitKind::VariableWidth,
        u64::from(variable.width),
        i32::MAX as u64,
      ));
    }
    let next_value_bytes = if let Some(alias) = alias {
      if self.variable(alias)? != variable {
        return Err(Error::IncompatibleAlias(alias));
      }
      self.value_bytes
    } else {
      self
        .value_bytes
        .checked_add(variable.width)
        .ok_or(Error::LimitExceeded(
          LimitKind::FrameBytes,
          u64::from(self.value_bytes) + u64::from(variable.width),
          u64::from(u32::MAX),
        ))?
    };
    let handle = Handle::new(unsafe {
      capi::fstWriterCreateVar(self.ctx, ty, dir, len, name.as_ptr(), alias.into_handle())
    })
    .ok_or(Error::BackendFailure(BackendOperation::CreateVariable))?;
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
  /// The current section is limited to a conservative one-GiB encoded budget,
  /// including 15 bytes per change, to protect libfst's 32-bit buffer arithmetic.
  pub fn emit_value_change(&mut self, handle: Handle, value: &[u8]) -> Result<()> {
    let variable = self.variable(handle)?;
    if variable.width == 0 {
      return Err(Error::ValueKindMismatch(
        handle,
        ValueKind::Variable,
        ValueKind::Fixed,
      ));
    }
    if value.len() != variable.width as usize {
      return Err(Error::ValueLengthMismatch(
        handle,
        variable.width as usize,
        value.len(),
      ));
    }
    let next_value_bytes = checked_value_bytes(self.buffered_value_bytes, value.len())?;
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
    self.buffered_value_bytes = next_value_bytes;
    self.check_status()
  }

  /// Emits variable-length value change for the given handle.
  ///
  /// Before the first explicit time change, the value is written at time zero.
  /// The same section budget as [`Self::emit_value_change`] applies. Exceeding
  /// it returns [`Error::LimitExceeded`] before changing the time or values.
  pub fn emit_var_len_value_change(&mut self, handle: Handle, value: &[u8]) -> Result<()> {
    if self.variable(handle)?.width != 0 {
      return Err(Error::ValueKindMismatch(
        handle,
        ValueKind::Fixed,
        ValueKind::Variable,
      ));
    }
    let next_value_bytes = checked_value_bytes(self.buffered_value_bytes, value.len())?;
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
        value.len() as u32,
      );
    }
    self.buffered_value_bytes = next_value_bytes;
    self.check_status()
  }

  /// Emits time change.
  ///
  /// Times must be nondecreasing; equal timestamps are allowed. Errors from
  /// libfst's public status flags are reported, but its fatal internal failures
  /// and errors during [`Drop`] cannot be converted to this result.
  pub fn emit_time_change(&mut self, time: u64) -> Result<()> {
    if let Some(last) = self.last_time.filter(|&last| time < last) {
      return Err(Error::TimeWentBackwards(last, time));
    }
    self.check_status()?;
    // Large hierarchies raise libfst's automatic flush threshold beyond this
    // wrapper's budget. Request an earlier section boundary while there is
    // still headroom for the next timestamp's changes.
    if self.buffered_value_bytes >= MAX_BUFFERED_VALUE_BYTES / 2 {
      self.flush();
    }
    // A public flush request is deferred until a time change. Only reset the
    // budget when that time change actually consumes the pending flush; the
    // same query also detects libfst's automatic section-size threshold.
    let flush_pending = unsafe { capi::fstWriterGetFlushContextPending(self.ctx) != 0 };
    unsafe { capi::fstWriterEmitTimeChange(self.ctx, time) };
    if flush_pending {
      self.buffered_value_bytes = 1;
    }
    self.last_time = Some(time);
    self.check_status()
  }

  /// Records whether waveform dumping is active at the current timestamp.
  ///
  /// Before the first explicit time change, the activity is recorded at zero.
  pub fn emit_dump_active(&mut self, enable: bool) -> Result<()> {
    self.check_status()?;
    if self.last_time.is_none() {
      self.emit_time_change(0)?;
    }
    unsafe { capi::fstWriterEmitDumpActive(self.ctx, enable as raw::c_int) };
    self.check_status()
  }

  /// Requests a flush on the next time change.
  ///
  /// libfst may ignore the request until several time changes have been emitted.
  /// Requesting a flush alone does not reset the value-change section budget.
  pub fn flush(&mut self) {
    unsafe { capi::fstWriterFlushContext(self.ctx) }
  }

  /// Returns storage metadata for a handle, rejecting handles outside the table.
  fn variable(&self, handle: Handle) -> Result<Variable> {
    self
      .variables
      .get(u32::from(handle) as usize - 1)
      .copied()
      .ok_or(Error::InvalidHandle(handle))
  }

  /// Reports a reached dump-size limit or seek failure from libfst's status flags.
  fn check_status(&self) -> Result<()> {
    if unsafe { capi::fstWriterGetDumpSizeLimitReached(self.ctx) != 0 } {
      Err(Error::DumpSizeLimitReached)
    } else if unsafe { capi::fstWriterGetFseekFailed(self.ctx) != 0 } {
      Err(Error::SeekFailed)
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

// Keep section payloads within one GiB, including record overhead. Upstream
// stores buffer sizes and offsets in u32 and passes some lengths to signed-int
// compression APIs. Its initial allocation is at most 2 GiB + 64 MiB. Growth
// happens only below the needed size (at most 1 GiB) and adds at most one record
// plus 64 MiB, so neither growth nor the record-offset calculations can wrap.
const MAX_BUFFERED_VALUE_BYTES: u32 = 1 << 30;

fn checked_value_bytes(buffered: u32, len: usize) -> Result<u32> {
  // A record must fit even in an empty section (one byte plus 15 bytes of
  // record overhead). Such a record cannot be rescued by flushing the section.
  let payload_limit = MAX_BUFFERED_VALUE_BYTES - 16;
  if len > payload_limit as usize {
    return Err(Error::LimitExceeded(
      LimitKind::ValueRecordBytes,
      len as u64,
      u64::from(payload_limit),
    ));
  }
  let size = u64::from(buffered) + len as u64 + 15;
  if size > u64::from(MAX_BUFFERED_VALUE_BYTES) {
    Err(Error::LimitExceeded(
      LimitKind::SectionValueBytes,
      size,
      u64::from(MAX_BUFFERED_VALUE_BYTES),
    ))
  } else {
    Ok(size as u32)
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_temp_dir::TestDir;

  #[test]
  fn value_change_budget_rejects_large_records_and_cumulative_overflow() {
    assert_eq!(checked_value_bytes(1, 0), Ok(16));
    assert_eq!(
      checked_value_bytes(1, MAX_BUFFERED_VALUE_BYTES as usize - 16),
      Ok(MAX_BUFFERED_VALUE_BYTES)
    );
    let payload_limit = u64::from(MAX_BUFFERED_VALUE_BYTES - 16);
    for len in [
      MAX_BUFFERED_VALUE_BYTES as usize - 15,
      u32::MAX as usize,
      usize::MAX,
    ] {
      assert_eq!(
        checked_value_bytes(1, len),
        Err(Error::LimitExceeded(
          LimitKind::ValueRecordBytes,
          len as u64,
          payload_limit
        ))
      );
    }
    for (buffered, len) in [
      (MAX_BUFFERED_VALUE_BYTES - 14, 0),
      (MAX_BUFFERED_VALUE_BYTES - 16, 2),
      (u32::MAX, 1),
    ] {
      assert_eq!(
        checked_value_bytes(buffered, len),
        Err(Error::LimitExceeded(
          LimitKind::SectionValueBytes,
          u64::from(buffered) + len as u64 + 15,
          u64::from(MAX_BUFFERED_VALUE_BYTES),
        ))
      );
    }
  }

  #[test]
  fn frame_limit_rejects_before_creating_a_variable() {
    let dir = TestDir::new();
    let file = dir.path("writer-frame-budget.fst");
    let mut writer = Writer::create(&file, true).unwrap();
    // Exercise frame-offset overflow without allocating a multi-GiB frame.
    writer.value_bytes = u32::MAX;
    assert_eq!(
      writer.create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "rejected", None),
      Err(Error::LimitExceeded(
        LimitKind::FrameBytes,
        u64::from(u32::MAX) + 1,
        u64::from(u32::MAX),
      ))
    );
    assert_eq!(writer.value_bytes, u32::MAX);
    assert!(writer.variables.is_empty());
    writer.value_bytes = 0;
    let bit = writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
      .unwrap();
    assert_eq!(u32::from(bit), 1);
    writer.emit_value_change(bit, b"1").unwrap();
    writer.emit_time_change(1).unwrap();
    drop(writer);
    let reader = crate::Reader::open(&file).unwrap();
    assert_eq!(reader.var_count(), 1);
  }

  #[test]
  fn exhausted_budget_rejects_without_mutation_and_resets_only_after_flush() {
    // Inject a full budget instead of allocating GiB-sized value buffers.
    let dir = TestDir::new();
    let file = dir.path("writer-budget.fst");
    let mut writer = Writer::create(&file, true).unwrap();
    let bit = writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
      .unwrap();
    let string = writer
      .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "string", None)
      .unwrap();
    writer.buffered_value_bytes = MAX_BUFFERED_VALUE_BYTES;
    assert_eq!(
      writer.emit_value_change(bit, b"1"),
      Err(Error::LimitExceeded(
        LimitKind::SectionValueBytes,
        u64::from(MAX_BUFFERED_VALUE_BYTES) + 16,
        u64::from(MAX_BUFFERED_VALUE_BYTES),
      ))
    );
    assert_eq!(
      writer.emit_var_len_value_change(string, b"first"),
      Err(Error::LimitExceeded(
        LimitKind::SectionValueBytes,
        u64::from(MAX_BUFFERED_VALUE_BYTES) + 20,
        u64::from(MAX_BUFFERED_VALUE_BYTES),
      ))
    );
    assert_eq!(writer.last_time, None);
    assert_eq!(writer.buffered_value_bytes, MAX_BUFFERED_VALUE_BYTES);

    writer.buffered_value_bytes = 1;
    writer.emit_value_change(bit, b"1").unwrap();
    writer.emit_var_len_value_change(string, b"first").unwrap();
    let budget_before_manual_flush = MAX_BUFFERED_VALUE_BYTES / 2 - 1;
    writer.buffered_value_bytes = budget_before_manual_flush;
    writer.emit_time_change(0).unwrap();
    assert_eq!(writer.buffered_value_bytes, budget_before_manual_flush);
    writer.emit_time_change(1).unwrap();
    writer.emit_time_change(2).unwrap();
    writer.flush();
    assert_eq!(writer.buffered_value_bytes, budget_before_manual_flush);
    writer.emit_time_change(3).unwrap();
    assert_eq!(writer.buffered_value_bytes, 1);
    writer.emit_value_change(bit, b"0").unwrap();
    writer.emit_var_len_value_change(string, b"last").unwrap();
    writer.emit_time_change(4).unwrap();
    drop(writer);

    let mut reader = crate::Reader::open(&file).unwrap();
    reader.set_mask_all();
    let mut values = Vec::new();
    reader
      .for_each_block(|time, handle, value, _| values.push((time, handle, value.to_vec())))
      .unwrap();
    // The second section can repeat a fixed-width checkpoint at its boundary.
    assert!(values.contains(&(0, bit, b"1".to_vec())));
    assert!(values.contains(&(0, string, b"first".to_vec())));
    assert!(values.contains(&(3, bit, b"0".to_vec())));
    assert!(values.contains(&(3, string, b"last".to_vec())));
    assert_eq!(values.iter().filter(|(time, _, _)| *time == 0).count(), 2);
  }

  #[test]
  fn time_changes_flush_large_budgets_when_the_engine_can_accept_the_request() {
    let dir = TestDir::new();
    let file = dir.path("writer-auto-budget.fst");
    let mut writer = Writer::create(&file, true).unwrap();
    let bit = writer
      .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
      .unwrap();
    writer.emit_value_change(bit, b"0").unwrap();
    writer.buffered_value_bytes = MAX_BUFFERED_VALUE_BYTES;

    // At the first two subsequent time changes, libfst ignores flush requests.
    // Do not reset the budget before its actual buffer is flushed.
    for time in [1, 2] {
      writer.emit_time_change(time).unwrap();
      assert_eq!(writer.buffered_value_bytes, MAX_BUFFERED_VALUE_BYTES);
      assert_eq!(
        writer.emit_value_change(bit, b"1"),
        Err(Error::LimitExceeded(
          LimitKind::SectionValueBytes,
          u64::from(MAX_BUFFERED_VALUE_BYTES) + 16,
          u64::from(MAX_BUFFERED_VALUE_BYTES),
        ))
      );
    }
    writer.emit_time_change(3).unwrap();
    assert_eq!(writer.buffered_value_bytes, 1);
    writer.emit_value_change(bit, b"1").unwrap();
    writer.emit_time_change(4).unwrap();
    drop(writer);

    let mut reader = crate::Reader::open(&file).unwrap();
    reader.set_mask_all();
    let mut values = Vec::new();
    reader
      .for_each_block(|time, _, value, _| values.push((time, value.to_vec())))
      .unwrap();
    assert!(values.contains(&(0, b"0".to_vec())));
    assert!(values.contains(&(3, b"1".to_vec())));
    assert!(
      !values
        .iter()
        .any(|(time, value)| *time < 3 && value == b"1")
    );
  }
}
