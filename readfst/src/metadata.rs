use crate::section::{Item, Section};
use fstapi::{Reader, Result, file_type};
use serde::Serialize;
use tabled::Tabled;

/// Metadata information.
#[derive(Serialize, Tabled)]
pub struct Metadata {
  #[tabled(rename = "Date")]
  date: String,
  #[tabled(rename = "Version")]
  version: String,
  #[tabled(rename = "File type")]
  file_type: &'static str,
  #[tabled(rename = "Timescale")]
  timescale: &'static str,
  #[tabled(rename = "Timezero")]
  #[serde(serialize_with = "crate::json::decimal")]
  timezero: i64,
  #[tabled(rename = "Start time")]
  #[serde(serialize_with = "crate::json::decimal")]
  start_time: u64,
  #[tabled(rename = "End time")]
  #[serde(serialize_with = "crate::json::decimal")]
  end_time: u64,
  #[tabled(rename = "Number of scopes")]
  #[serde(serialize_with = "crate::json::decimal")]
  num_scopes: u64,
  #[tabled(rename = "Number of variables")]
  #[serde(serialize_with = "crate::json::decimal")]
  num_vars: u64,
  #[tabled(rename = "Number of alias")]
  #[serde(serialize_with = "crate::json::decimal")]
  num_aliases: u64,
}

impl Metadata {
  pub fn new(reader: &Reader) -> Result<Self> {
    Ok(Self {
      date: reader.date()?.trim().into(),
      version: reader.version()?.trim().into(),
      file_type: match reader.file_type() {
        file_type::VERILOG => "Verilog",
        file_type::VHDL => "VHDL",
        file_type::VERILOG_VHDL => "Verilog/VHDL",
        _ => "Unknown",
      },
      timescale: match reader.timescale_str() {
        Some(t) => t,
        None => "Unknown",
      },
      timezero: reader.timezero(),
      start_time: reader.start_time(),
      end_time: reader.end_time(),
      num_scopes: reader.scope_count(),
      num_vars: reader.var_count(),
      num_aliases: reader.alias_count(),
    })
  }
}

impl Section for Metadata {
  type Item = Self;

  fn name() -> &'static str {
    "Metadata"
  }

  fn item(&self) -> Item<'_, Self::Item> {
    Item::One(self)
  }
}
