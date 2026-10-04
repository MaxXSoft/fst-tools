//! Record encodings and stdout budgets shared by text and structured searches.

use crate::checker::VarInfo;
use crate::find::Scan;
use crate::printer::{FullPrinter, NamePrinter, Printer};
use crate::{Cli, Error, Format, Result};
use fstapi::{Handle, Reader, VarType, var_type};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;

pub(crate) const MIN_BYTES: u64 = 4096;
const FOOTER_RESERVE: u64 = 2048;

/// Metadata for a selected physical signal, retaining the legacy display name.
struct Signal {
  name: String,
  canonical_name: String,
  aliases: Vec<String>,
  width: u32,
  ty: VarType,
}

impl Signal {
  /// Legacy matching sees decimal real callbacks, never native double bytes.
  fn encoding(&self) -> &'static str {
    match self.ty {
      var_type::GEN_STRING => "bytes_hex",
      var_type::VCD_REAL
      | var_type::VCD_REAL_PARAMETER
      | var_type::VCD_REALTIME
      | var_type::SV_SHORTREAL => "real_decimal",
      _ if self.width == 0 => "bytes_hex",
      var_type::VCD_PORT => "evcd",
      _ => "bits",
    }
  }
}

/// Deterministic physical-handle metadata, built only for JSON output.
#[derive(Default)]
pub(crate) struct Catalog(BTreeMap<Handle, Signal>);

impl Catalog {
  pub(crate) fn new(reader: &mut Reader, vars: &VarInfo) -> Result<Self> {
    let mut signals = BTreeMap::<Handle, Signal>::new();
    for entry in reader.vars() {
      let (path, var) = entry?;
      let Some(name) = vars.name(var.handle()) else {
        continue;
      };
      if var.is_alias() {
        signals
          .get_mut(&var.handle())
          .ok_or_else(|| Error::Arguments("alias precedes its physical signal".into()))?
          .aliases
          .push(path);
      } else {
        signals.insert(
          var.handle(),
          Signal {
            name: name.into(),
            canonical_name: path,
            aliases: Vec::new(),
            width: var.length(),
            ty: var.ty(),
          },
        );
      }
    }
    Ok(Self(signals))
  }
}

/// Serialize one entire line before deciding whether it fits the byte budget.
fn json_line(record: &Value) -> Result<Vec<u8>> {
  let mut bytes = serde_json::to_vec(record)?;
  bytes.push(b'\n');
  Ok(bytes)
}

/// Hex preserves arbitrary string callback bytes without UTF-8 replacement.
fn hex(bytes: &[u8]) -> String {
  const HEX: &[u8] = b"0123456789abcdef";
  let mut result = String::with_capacity(bytes.len() * 2);
  for &byte in bytes {
    result.push(HEX[usize::from(byte >> 4)] as char);
    result.push(HEX[usize::from(byte & 15)] as char);
  }
  result
}

/// Keep complete records and reserve space for a truthful final status.
pub(crate) struct Output<'a, W> {
  writer: &'a mut W,
  cli: &'a Cli,
  catalog: &'a Catalog,
  written: u64,
  emitted: u64,
  omitted: u64,
  omitted_metadata: u64,
  matches_blocked: bool,
}

impl<'a, W: Write> Output<'a, W> {
  pub(crate) fn new(
    writer: &'a mut W,
    cli: &'a Cli,
    reader: &Reader,
    catalog: &'a Catalog,
    start: u64,
    end: u64,
  ) -> Result<Self> {
    let mut output = Self {
      writer,
      cli,
      catalog,
      written: 0,
      emitted: 0,
      omitted: 0,
      omitted_metadata: 0,
      matches_blocked: false,
    };
    if cli.output_format() != Format::Json {
      return Ok(output);
    }
    let header = json_line(&json!({
      "type": "header", "schema": "findfst", "schema_version": 1,
      "file": cli.file, "trace_start": reader.start_time().to_string(),
      "trace_end": reader.end_time().to_string(), "start": start.to_string(), "end": end.to_string(),
      "timescale_exponent": reader.timescale(), "timezero": reader.timezero().to_string(),
      "interval": "inclusive", "match_semantics": "in_range_callbacks_only",
      "mode": if cli.all_matches { "all_matches" } else { "first_per_handle" },
      "value_pattern": cli.value, "value_regex": cli.regex, "hex": cli.hex,
      "event_order": "libfst_callback_order", "real_values": "libfst_default_decimal",
      "limits": {
        "max_rows": cli.max_rows.map(|n| n.to_string()),
        "max_bytes": cli.max_bytes.map(|n| n.to_string()),
        "max_callbacks": cli.max_callbacks.map(|n| n.to_string()),
        "max_duration_ms": cli.max_duration_ms.map(|n| n.to_string()),
      },
    }))?;
    if !output.fits(header.len() as u64) {
      return Err(Error::Arguments(
        "--max-bytes cannot fit the header and reserved footer".into(),
      ));
    }
    output.write(&header)?;
    for (&handle, signal) in &catalog.0 {
      output.metadata(&json!({
        "type": "signal", "handle": u32::from(handle), "name": signal.name,
        "canonical_name": signal.canonical_name, "aliases": signal.aliases,
        "width": signal.width, "var_type": signal.ty, "encoding": signal.encoding(),
      }))?;
    }
    for (time, active) in reader.dump_activity() {
      output
        .metadata(&json!({"type": "dump_activity", "time": time.to_string(), "active": active}))?;
    }
    Ok(output)
  }

  /// The footer reservation applies only when JSON has a stdout footer.
  fn fits(&self, bytes: u64) -> bool {
    self.cli.max_bytes.is_none_or(|limit| {
      let reserve = if self.cli.output_format() == Format::Json {
        FOOTER_RESERVE
      } else {
        0
      };
      self.written.saturating_add(bytes) <= limit.saturating_sub(reserve)
    })
  }

  fn write(&mut self, bytes: &[u8]) -> Result<()> {
    self.writer.write_all(bytes)?;
    self.written += bytes.len() as u64;
    Ok(())
  }

  /// Oversized metadata is omitted whole, never emitted as partial JSON.
  fn metadata(&mut self, record: &Value) -> Result<()> {
    let bytes = json_line(record)?;
    if self.fits(bytes.len() as u64) {
      self.write(&bytes)
    } else {
      self.omitted_metadata += 1;
      Ok(())
    }
  }

  /// Count every eligible match even after the emitted prefix hits a limit.
  pub(crate) fn matched(
    &mut self,
    time: u64,
    handle: Handle,
    name: &str,
    value: &[u8],
  ) -> Result<()> {
    if self.matches_blocked || self.cli.max_rows.is_some_and(|limit| self.emitted >= limit) {
      self.matches_blocked = true;
      self.omitted += 1;
      return Ok(());
    }
    let bytes = if self.cli.output_format() == Format::Json {
      let signal = &self.catalog.0[&handle];
      let (encoding, value) = match signal.encoding() {
        "bytes_hex" => ("bytes_hex", hex(value)),
        encoding => match std::str::from_utf8(value) {
          Ok(value) => (encoding, value.to_owned()),
          Err(_) => ("bytes_hex", hex(value)),
        },
      };
      json_line(&json!({
        "type": "match", "sequence": self.emitted.to_string(), "time": time.to_string(),
        "handle": u32::from(handle), "name": name, "width": signal.width,
        "var_type": signal.ty, "encoding": encoding, "value": value,
      }))?
    } else {
      let mut bytes = Vec::new();
      if self.cli.names_only {
        NamePrinter.print(&mut bytes, time, name, value)?;
      } else {
        FullPrinter.print(&mut bytes, time, name, value)?;
      }
      bytes
    };
    if self.fits(bytes.len() as u64) {
      self.write(&bytes)?;
      self.emitted += 1;
    } else {
      self.matches_blocked = true;
      self.omitted += 1;
    }
    Ok(())
  }

  /// Exact totals are available only after traversal completes normally.
  pub(crate) fn finish(&mut self, scan: &Scan, selected: usize) -> Result<()> {
    let truncated = self.omitted != 0 || self.omitted_metadata != 0;
    if self.cli.output_format() == Format::Text {
      if !scan.complete || truncated {
        eprintln!(
          "findfst: status={}, output_truncated={}, observed_matches={}, emitted_matches={}, observed_omitted_matches={}, stop_reason={}",
          if scan.complete { "complete" } else { "partial" },
          truncated,
          self.emitted + self.omitted,
          self.emitted,
          self.omitted,
          scan.stop_reason.unwrap_or("none")
        );
      }
      return Ok(());
    }
    let mut footer = json!({
      "type": "summary", "status": if scan.complete { "complete" } else { "partial" },
      "execution_complete": scan.complete, "stop_reason": scan.stop_reason,
      "complete": scan.complete, "reason": scan.stop_reason, "unprocessed_input": !scan.complete,
      "last_callback_time": scan.last_callback_time.map(|time| time.to_string()),
      "processed_through": scan.processed_through.map(|time| time.to_string()),
      "output_truncated": truncated, "metadata_complete": self.omitted_metadata == 0,
      "omitted_metadata_records": self.omitted_metadata.to_string(),
      "selected_handles": selected, "decoded_callbacks": scan.callbacks.to_string(),
      "observed_matches": (self.emitted + self.omitted).to_string(),
      "total_matches": scan.complete.then(|| (self.emitted + self.omitted).to_string()),
      "emitted_matches": self.emitted.to_string(), "observed_omitted_matches": self.omitted.to_string(),
      "total_omitted_matches": scan.complete.then(|| self.omitted.to_string()),
      "stdout_bytes": "0",
    });
    // The byte count includes its own decimal representation and the newline.
    let bytes = loop {
      let bytes = json_line(&footer)?;
      let total = (self.written + bytes.len() as u64).to_string();
      if footer["stdout_bytes"] == total {
        break bytes;
      }
      footer["stdout_bytes"] = total.into();
    };
    if bytes.len() as u64 > FOOTER_RESERVE {
      return Err(Error::Arguments(
        "internal footer exceeds reserved output space".into(),
      ));
    }
    self.write(&bytes)
  }
}
