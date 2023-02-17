use crate::section::{Item, Section};
use fstapi::{array_type, attr_type, enum_value_type, misc_type, pack_type};
use fstapi::{Attr, Hier, Reader, Result};
use tabled::Tabled;

/// Attribute information.
#[derive(Tabled)]
pub struct AttrInfo {
  #[tabled(rename = "Type")]
  ty: &'static str,
  #[tabled(rename = "Subtype")]
  subtype: &'static str,
  #[tabled(rename = "Name")]
  name: String,
  #[tabled(rename = "Arg")]
  arg: u64,
  #[tabled(rename = "Arg From Name")]
  arg_from_name: u64,
}

impl AttrInfo {
  fn new(attr: Attr) -> Result<Self> {
    let (ty, subtype) = match attr.ty() {
      attr_type::MISC => (
        "Misc",
        match attr.subtype() {
          misc_type::COMMENT => "Comment",
          misc_type::ENVVAR => "EnvVar",
          misc_type::SUPVAR => "SupVar",
          misc_type::PATHNAME => "PathName",
          misc_type::SOURCESTEM => "SourceStem",
          misc_type::SOURCEISTEM => "SourceIStem",
          misc_type::VALUELIST => "ValueList",
          misc_type::ENUMTABLE => "EnumTable",
          misc_type::UNKNOWN => "Unknown",
          _ => unreachable!(),
        },
      ),
      attr_type::ARRAY => (
        "Array",
        match attr.subtype() {
          array_type::NONE => "None",
          array_type::UNPACKED => "Unpacked",
          array_type::PACKED => "Packed",
          array_type::SPARSE => "Sparse",
          _ => unreachable!(),
        },
      ),
      attr_type::ENUM => (
        "Enum",
        match attr.subtype() {
          enum_value_type::SV_INTEGER => "SvInteger",
          enum_value_type::SV_BIT => "SvBit",
          enum_value_type::SV_LOGIC => "SvLogic",
          enum_value_type::SV_INT => "SvInt",
          enum_value_type::SV_SHORTINT => "SvShortint",
          enum_value_type::SV_LONGINT => "SvLongint",
          enum_value_type::SV_BYTE => "SvByte",
          enum_value_type::SV_UNSIGNED_INTEGER => "SvUnsignedInteger",
          enum_value_type::SV_UNSIGNED_BIT => "SvUnsignedBit",
          enum_value_type::SV_UNSIGNED_LOGIC => "SvUnsignedLogic",
          enum_value_type::SV_UNSIGNED_INT => "SvUnsignedInt",
          enum_value_type::SV_UNSIGNED_SHORTINT => "SvUnsignedShortint",
          enum_value_type::SV_UNSIGNED_LONGINT => "SvUnsignedLongint",
          enum_value_type::SV_UNSIGNED_BYTE => "SvUnsignedByte",
          enum_value_type::REG => "Reg",
          enum_value_type::TIME => "Time",
          _ => unreachable!(),
        },
      ),
      attr_type::PACK => (
        "Pack",
        match attr.subtype() {
          pack_type::NONE => "None",
          pack_type::UNPACKED => "Unpacked",
          pack_type::PACKED => "Packed",
          pack_type::TAGGED_PACKED => "TaggedPacked",
          _ => unreachable!(),
        },
      ),
      _ => unreachable!(),
    };
    Ok(Self {
      ty,
      subtype,
      name: attr.name()?.into(),
      arg: attr.arg(),
      arg_from_name: attr.arg_from_name(),
    })
  }
}

/// Attributes information
pub struct Attrs {
  attrs: Vec<AttrInfo>,
}

impl Attrs {
  pub fn new(reader: &mut Reader) -> Result<Self> {
    let mut attrs = Vec::new();
    for hier in reader.hiers() {
      if let Hier::AttrBegin(attr) = hier {
        attrs.push(AttrInfo::new(attr)?);
      }
    }
    Ok(Self { attrs })
  }
}

impl Section for Attrs {
  type Item = AttrInfo;

  fn name() -> &'static str {
    "Attributes"
  }

  fn item(&self) -> Item<Self::Item> {
    Item::Many(&self.attrs)
  }
}
