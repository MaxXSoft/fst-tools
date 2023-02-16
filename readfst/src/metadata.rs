use crate::section::{Item, Section};
use fstapi::{file_type, Reader, Result};
use tabled::Tabled;

/// Metadata information.
#[derive(Tabled)]
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
  timezero: i64,
  #[tabled(rename = "Start type")]
  start_time: u64,
  #[tabled(rename = "End type")]
  end_time: u64,
  #[tabled(rename = "Number of scopes")]
  num_scopes: u64,
  #[tabled(rename = "Number of variables")]
  num_vars: u64,
  #[tabled(rename = "Number of alias")]
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

  fn name(&self) -> &str {
    "Metadata"
  }

  fn item(&mut self) -> Item<&Self::Item> {
    Item::One(self)
  }
}
