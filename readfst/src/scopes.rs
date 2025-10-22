use crate::section::{Item, Section};
use fstapi::{Hier, Reader, Result, Scope, scope_type};
use tabled::Tabled;

/// Scope information.
#[derive(Tabled)]
pub struct ScopeInfo {
  #[tabled(rename = "Type")]
  ty: &'static str,
  #[tabled(rename = "Name")]
  name: String,
  #[tabled(rename = "Component")]
  component: String,
}

impl ScopeInfo {
  fn new(scope: Scope) -> Result<Self> {
    Ok(Self {
      ty: match scope.ty() {
        scope_type::VCD_MODULE => "VcdModule",
        scope_type::VCD_TASK => "VcdTask",
        scope_type::VCD_FUNCTION => "VcdFunction",
        scope_type::VCD_BEGIN => "VcdBegin",
        scope_type::VCD_FORK => "VcdFork",
        scope_type::VCD_GENERATE => "VcdGenerate",
        scope_type::VCD_STRUCT => "VcdStruct",
        scope_type::VCD_UNION => "VcdUnion",
        scope_type::VCD_CLASS => "VcdClass",
        scope_type::VCD_INTERFACE => "VcdInterface",
        scope_type::VCD_PACKAGE => "VcdPackage",
        scope_type::VCD_PROGRAM => "VcdProgram",
        scope_type::VHDL_ARCHITECTURE => "VhdlArchitecture",
        scope_type::VHDL_PROCEDURE => "VhdlProcedure",
        scope_type::VHDL_FUNCTION => "VhdlFunction",
        scope_type::VHDL_RECORD => "VhdlRecord",
        scope_type::VHDL_PROCESS => "VhdlProcess",
        scope_type::VHDL_BLOCK => "VhdlBlock",
        scope_type::VHDL_FOR_GENERATE => "VhdlForGenerate",
        scope_type::VHDL_IF_GENERATE => "VhdlIfGenerate",
        scope_type::VHDL_GENERATE => "VhdlGenerate",
        scope_type::VHDL_PACKAGE => "VhdlPackage",
        _ => unreachable!(),
      },
      name: scope.name()?.into(),
      component: scope.component()?.into(),
    })
  }
}

/// Scopes information.
pub struct Scopes {
  scopes: Vec<ScopeInfo>,
}

impl Scopes {
  pub fn new(reader: &mut Reader) -> Result<Self> {
    let mut scopes = Vec::new();
    for hier in reader.hiers() {
      if let Hier::Scope(scope) = hier {
        scopes.push(ScopeInfo::new(scope)?);
      }
    }
    Ok(Self { scopes })
  }
}

impl Section for Scopes {
  type Item = ScopeInfo;

  fn name() -> &'static str {
    "Scopes"
  }

  fn item(&self) -> Item<'_, Self::Item> {
    Item::Many(&self.scopes)
  }
}
