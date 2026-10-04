use crate::section::{Item, Print, Section, ToTable};
use fstapi::{Handle, Reader, Result, Var, var_dir, var_type};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use tabled::object::{FirstRow, LastColumn};
use tabled::{Alignment, Disable, Modify, Panel, Tabled};

/// Variable name map.
type VarNames = HashMap<Handle, Box<str>>;

/// Variable information.
#[derive(Serialize, Tabled)]
pub struct VarInfo {
  #[tabled(rename = "Handle")]
  #[serde(serialize_with = "crate::json::handle")]
  handle: Handle,
  #[tabled(rename = "Type")]
  #[serde(rename = "type")]
  ty: &'static str,
  #[tabled(rename = "Direction")]
  direction: &'static str,
  #[tabled(rename = "Name")]
  name: String,
  #[tabled(rename = "Length in Bits")]
  #[serde(rename = "width")]
  length: u32,
  #[tabled(rename = "Alias Of")]
  #[serde(skip)]
  alias_of: String,
}

impl VarInfo {
  pub(crate) fn new(name: &str, var: &Var, alias_of: &str) -> Self {
    Self {
      handle: var.handle(),
      ty: match var.ty() {
        var_type::VCD_EVENT => "VcdEvent",
        var_type::VCD_INTEGER => "VcdInteger",
        var_type::VCD_PARAMETER => "VcdParameter",
        var_type::VCD_REAL => "VcdReal",
        var_type::VCD_REAL_PARAMETER => "VcdRealParameter",
        var_type::VCD_REG => "VcdReg",
        var_type::VCD_SUPPLY0 => "VcdSupply0",
        var_type::VCD_SUPPLY1 => "VcdSupply1",
        var_type::VCD_TIME => "VcdTime",
        var_type::VCD_TRI => "VcdTri",
        var_type::VCD_TRIAND => "VcdTriand",
        var_type::VCD_TRIOR => "VcdTrior",
        var_type::VCD_TRIREG => "VcdTrireg",
        var_type::VCD_TRI0 => "VcdTri0",
        var_type::VCD_TRI1 => "VcdTri1",
        var_type::VCD_WAND => "VcdWand",
        var_type::VCD_WIRE => "VcdWire",
        var_type::VCD_WOR => "VcdWor",
        var_type::VCD_PORT => "VcdPort",
        var_type::VCD_SPARRAY => "VcdSparray",
        var_type::VCD_REALTIME => "VcdRealtime",
        var_type::GEN_STRING => "GenString",
        var_type::SV_BIT => "SvBit",
        var_type::SV_LOGIC => "SvLogic",
        var_type::SV_INT => "SvInt",
        var_type::SV_SHORTINT => "SvShortint",
        var_type::SV_LONGINT => "SvLongint",
        var_type::SV_BYTE => "SvByte",
        var_type::SV_ENUM => "SvEnum",
        var_type::SV_SHORTREAL => "SvShortreal",
        _ => unreachable!(),
      },
      direction: match var.direction() {
        var_dir::IMPLICIT => "Implicit",
        var_dir::INPUT => "Input",
        var_dir::OUTPUT => "Output",
        var_dir::INOUT => "Inout",
        var_dir::BUFFER => "Buffer",
        var_dir::LINKAGE => "Linkage",
        _ => unreachable!(),
      },
      name: name.into(),
      length: var.length(),
      alias_of: alias_of.into(),
    }
  }
}

/// Trait for variable section.
pub trait VarSection: Sized {
  fn new(reader: &mut Reader, filter: Option<&Regex>) -> Result<Self>;
  fn vars(&self) -> &[VarInfo];
}

/// Variables information.
pub struct Variables {
  vars: Vec<VarInfo>,
}

impl Variables {
  fn new(reader: &mut Reader, no_aliases: bool, filter: Option<&Regex>) -> Result<Self> {
    let mut vars = Vec::new();
    let mut names = VarNames::new();
    for var in reader.vars() {
      let (name, var) = var?;
      // Retain canonical names even when only an alias matches the filter.
      if !var.is_alias() {
        assert!(names.insert(var.handle(), name.clone().into()).is_none());
      }
      if no_aliases && var.is_alias() {
        continue;
      }
      if filter.is_some_and(|filter| !filter.is_match(&name)) {
        continue;
      }
      // Collect variable information.
      let alias_of = if var.is_alias() {
        names[&var.handle()].as_ref()
      } else {
        ""
      };
      vars.push(VarInfo::new(&name, &var, alias_of));
    }
    Ok(Self { vars })
  }
}

impl VarSection for Variables {
  fn new(reader: &mut Reader, filter: Option<&Regex>) -> Result<Self> {
    Self::new(reader, false, filter)
  }

  fn vars(&self) -> &[VarInfo] {
    &self.vars
  }
}

impl Section for Variables {
  type Item = VarInfo;

  fn name() -> &'static str {
    "Variables"
  }

  fn item(&self) -> Item<'_, Self::Item> {
    Item::Many(&self.vars)
  }
}

/// Variables information without aliases.
pub struct NoAliasesVars(Variables);

impl VarSection for NoAliasesVars {
  fn new(reader: &mut Reader, filter: Option<&Regex>) -> Result<Self> {
    Variables::new(reader, true, filter).map(Self)
  }

  fn vars(&self) -> &[VarInfo] {
    self.0.vars()
  }
}

impl ToTable for NoAliasesVars {
  fn to_table(&self) -> tabled::Table {
    let mut table = self.0.to_table();
    if self.0.vars().is_empty() {
      return table;
    }
    table
      .with(Disable::row(FirstRow))
      .with(Disable::column(LastColumn))
      .with(Panel::header(Variables::name()))
      .with(Modify::new(FirstRow).with(Alignment::center()));
    table
  }
}

/// Variables information with names only.
pub struct NameOnly<V>(V);

impl<V> VarSection for NameOnly<V>
where
  V: VarSection,
{
  fn new(reader: &mut Reader, filter: Option<&Regex>) -> Result<Self> {
    V::new(reader, filter).map(Self)
  }

  fn vars(&self) -> &[VarInfo] {
    self.0.vars()
  }
}

impl<V> Print for NameOnly<V>
where
  V: VarSection,
{
  fn print(&self) {
    for var in self.vars() {
      println!("{}", var.name);
    }
  }
}

/// Variables information with names only and with aliases.
pub type NameOnlyVars = NameOnly<Variables>;

/// Variables information with names only and without aliases.
pub type NameOnlyNoAliasesVars = NameOnly<NoAliasesVars>;
