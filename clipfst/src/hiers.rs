use fstapi::{Error, Handle, Hier, Reader, Result, Scope, ScopeType, Writer};
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

/// Builds hierarchies for the output waveform.
///
/// Returns mappings of input handles to output handles.
pub fn build(
  reader: &mut Reader,
  writer: &mut Writer,
  re: Option<Regex>,
  strip_attrs: bool,
) -> Result<HashMap<Handle, Handle>> {
  let mut scopes = Vec::new();
  let mut handles = HashMap::new();
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
        let handle = writer.create_var(
          v.ty(),
          v.direction(),
          v.length(),
          name,
          handles.get(&v.handle()).copied(),
        )?;
        // Update mappings between input handles and output handles.
        handles.insert(v.handle(), handle);
      }

      // Write attributes only when `strip_attrs` is `false`.
      Hier::AttrBegin(a) if !strip_attrs => {
        writer.set_attr_begin(a.ty(), a.subtype() as i32, a.name()?, a.arg())?
      }
      Hier::AttrEnd if !strip_attrs => writer.set_attr_end(),
      _ => {}
    }
  }
  Ok(handles)
}
