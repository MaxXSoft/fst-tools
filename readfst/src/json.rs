//! Versioned, lossless metadata and hierarchy output for machine consumers.

use crate::{Cli, attrs::AttrInfo, metadata::Metadata, scopes::ScopeInfo, vars::VarInfo};
use fstapi::{Handle, Hier, Reader, Result};
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::Display;

/// Serialize potentially large integers without losing precision in JSON clients.
pub(crate) fn decimal<T: Display, S: Serializer>(
  value: &T,
  serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
  serializer.collect_str(value)
}

/// Handles are bounded unsigned 32-bit numbers, safe in JSON numeric clients.
pub(crate) fn handle<S: Serializer>(
  value: &Handle,
  serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
  serializer.serialize_u32((*value).into())
}

/// One complete JSON response. Unrequested sections are omitted.
#[derive(Serialize)]
pub(crate) struct Document {
  schema: &'static str,
  schema_version: u32,
  file: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  metadata: Option<JsonMetadata>,
  #[serde(skip_serializing_if = "Option::is_none")]
  variables: Option<Vec<JsonVariable>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  scopes: Option<Vec<JsonScope>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  attributes: Option<Vec<JsonAttribute>>,
}

/// Header information with the base-10 seconds exponent needed to interpret ticks.
#[derive(Serialize)]
struct JsonMetadata {
  #[serde(flatten)]
  info: Metadata,
  timescale_exponent: i32,
  file_type_code: u32,
}

/// Each declaration retains its own path, including aliases sharing one handle.
#[derive(Serialize)]
struct JsonVariable {
  #[serde(flatten)]
  info: VarInfo,
  #[serde(serialize_with = "decimal")]
  hierarchy_index: usize,
  path: Vec<String>,
  alias_of: Option<String>,
  canonical_name: String,
  is_alias: bool,
  type_code: u32,
  direction_code: u32,
}

/// Scope paths disambiguate repeated local names in different parents.
#[derive(Serialize)]
struct JsonScope {
  #[serde(flatten)]
  info: ScopeInfo,
  #[serde(serialize_with = "decimal")]
  hierarchy_index: usize,
  full_name: String,
  path: Vec<String>,
  type_code: u32,
}

/// Preserve begin/end boundaries and hierarchy position without guessing owners.
#[derive(Serialize)]
struct JsonAttribute {
  #[serde(serialize_with = "decimal")]
  hierarchy_index: usize,
  scope_path: Vec<String>,
  event: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  data: Option<AttrInfo>,
}

impl Document {
  /// Read selected sections in one hierarchy pass without decoding value changes.
  pub(crate) fn new(reader: &mut Reader, cli: &Cli) -> Result<Self> {
    let mut result = Self {
      schema: "readfst",
      schema_version: 1,
      file: cli.file.clone(),
      metadata: if cli.metadata {
        Some(JsonMetadata {
          info: Metadata::new(reader)?,
          timescale_exponent: reader.timescale(),
          file_type_code: reader.file_type(),
        })
      } else {
        None
      },
      variables: cli.vars.then(Vec::new),
      scopes: cli.scopes.then(Vec::new),
      attributes: cli.attrs.then(Vec::new),
    };
    if !cli.vars && !cli.scopes && !cli.attrs {
      return Ok(result);
    }

    let mut path = Vec::new();
    let mut canonical_names = HashMap::<Handle, String>::new();
    for (hierarchy_index, hier) in reader.hiers().enumerate() {
      match hier {
        Hier::Scope(scope) => {
          path.push(scope.name()?.to_owned());
          if let Some(scopes) = result.scopes.as_mut() {
            scopes.push(JsonScope {
              type_code: scope.ty(),
              info: ScopeInfo::new(scope)?,
              hierarchy_index,
              full_name: path.join("."),
              path: path.clone(),
            });
          }
        }
        Hier::Upscope => {
          path.pop();
        }
        Hier::Var(var) => {
          let Some(variables) = result.variables.as_mut() else {
            continue;
          };
          let mut var_path = path.clone();
          var_path.push(var.name()?.to_owned());
          let name = var_path.join(".");
          if !var.is_alias() {
            canonical_names.insert(var.handle(), name.clone());
          }
          if (cli.no_aliases && var.is_alias())
            || cli
              .signals
              .as_ref()
              .is_some_and(|filter| !filter.is_match(&name))
          {
            continue;
          }
          let canonical_name = canonical_names
            .get(&var.handle())
            .ok_or(fstapi::Error::InvalidHierarchy(
              canonical_names.len() as u64 + 1,
              var.handle().into(),
            ))?
            .clone();
          variables.push(JsonVariable {
            info: VarInfo::new(
              &name,
              &var,
              if var.is_alias() { &canonical_name } else { "" },
            ),
            hierarchy_index,
            path: var_path,
            alias_of: var.is_alias().then(|| canonical_name.clone()),
            canonical_name,
            is_alias: var.is_alias(),
            type_code: var.ty(),
            direction_code: var.direction(),
          });
        }
        Hier::AttrBegin(attr) => {
          if let Some(attributes) = result.attributes.as_mut() {
            attributes.push(JsonAttribute {
              hierarchy_index,
              scope_path: path.clone(),
              event: "begin",
              data: Some(AttrInfo::new(attr)?),
            });
          }
        }
        Hier::AttrEnd => {
          if let Some(attributes) = result.attributes.as_mut() {
            attributes.push(JsonAttribute {
              hierarchy_index,
              scope_path: path.clone(),
              event: "end",
              data: None,
            });
          }
        }
      }
    }
    Ok(result)
  }
}
