use fstapi::{Error, Handle, Hier, LimitKind, Reader, Result, Scope, ScopeType, Writer, var_type};
use regex::Regex;
use std::collections::HashMap;

struct ScopeStorage {
  ty: ScopeType,
  name: String,
  component: String,
  visited: bool,
}

impl TryFrom<Scope<'_>> for ScopeStorage {
  type Error = Error;

  fn try_from(s: Scope) -> Result<Self> {
    Ok(Self {
      ty: s.ty(),
      name: s.name()?.into(),
      component: s.component()?.into(),
      visited: false,
    })
  }
}

/// Selected input/output handle mappings and their storage requirements.
pub struct Selection {
  pub handles: HashMap<Handle, Handle>,
  pub has_variable_values: bool,
}

/// Builds hierarchies for the output waveform and returns the selected handles.
pub fn build(
  reader: &mut Reader,
  writer: &mut Writer,
  re: Option<Regex>,
  strip_attrs: bool,
) -> Result<Selection> {
  let mut scopes = Vec::new();
  let mut handles = HashMap::new();
  let mut has_variable_values = false;
  // Iterate over hierarchies of the input waveform.
  for hier in reader.hiers() {
    match hier {
      // If need to match signals, just store the scope.
      Hier::Scope(s) if re.is_some() => scopes.push(ScopeStorage::try_from(s)?),
      // Otherwise, write the current scope to the output.
      Hier::Scope(s) => writer.set_scope(s.ty(), s.name()?, s.component()?)?,

      // If no need to match signals, or there is a visited scope storage
      // (which means there are matching signals in this scope)
      // write the upscope to the output. Otherwise nothing to do.
      Hier::Upscope if re.is_none() || matches!(scopes.pop(), Some(s) if s.visited) => {
        writer.set_upscope()
      }

      Hier::Var(v) => {
        let name = v.name()?;
        // If need to match signals, check if the current signal matches.
        if let Some(re) = &re {
          if !re.is_match(name) {
            continue;
          }
          // Visit all unvisited scopes and write them to file.
          for s in scopes.iter_mut().filter(|s| !s.visited) {
            s.visited = true;
            writer.set_scope(s.ty, &s.name, &s.component)?;
          }
        }
        // Write the current variable to the output.
        has_variable_values |= v.ty() == var_type::GEN_STRING || v.length() == 0;
        // The hierarchy reports the EVCD port bit width, while its writer API
        // accepts the value plus two strength vectors and their separators.
        let length = if v.ty() == var_type::VCD_PORT {
          let encoded = u64::from(v.length()) * 3 + 2;
          u32::try_from(encoded).map_err(|_| {
            Error::LimitExceeded(LimitKind::EvcdEncodedWidth, encoded, u64::from(u32::MAX))
          })?
        } else {
          v.length()
        };
        let handle = writer.create_var(
          v.ty(),
          v.direction(),
          length,
          name,
          handles.get(&v.handle()).copied(),
        )?;
        // Update mappings between input handles and output handles.
        handles.insert(v.handle(), handle);
      }

      // Write attributes only when `strip_attrs` is `false`.
      Hier::AttrBegin(a) if !strip_attrs => {
        writer.set_attr_begin_raw(a.ty(), a.subtype(), a.name_cstr(), a.arg())?
      }
      Hier::AttrEnd if !strip_attrs => writer.set_attr_end(),
      _ => {}
    }
  }
  Ok(Selection {
    handles,
    has_variable_values,
  })
}
