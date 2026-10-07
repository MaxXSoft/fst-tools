//! Resolve declarations once, with one state slot per SQL binding.
use crate::error::Result;
use fstapi::{Handle, Reader};
use std::collections::{BTreeMap, HashMap};

pub(super) struct ResolvedSignals {
  pub schema: HashMap<String, (usize, u32)>,
  pub mapping: HashMap<Handle, Vec<usize>>,
  pub tick_index: usize,
  pub sample_index: usize,
}

pub(super) fn resolve(
  reader: &mut Reader,
  bindings: &BTreeMap<String, String>,
) -> Result<ResolvedSignals> {
  let names: Vec<_> = bindings.keys().cloned().collect();
  if names
    .iter()
    .any(|n| matches!(n.as_str(), "tick" | "sample_index"))
  {
    return Err("binding names tick and sample_index are reserved".into());
  }
  let mut paths: HashMap<&str, Vec<usize>> = HashMap::new();
  for (i, name) in names.iter().enumerate() {
    paths.entry(bindings[name].as_str()).or_default().push(i);
  }
  let mut mapping = HashMap::new();
  let mut widths = vec![0; names.len()];
  let mut found = vec![false; names.len()];
  for entry in reader.vars() {
    let (path, var) = entry?;
    if let Some(indices) = paths.get(path.as_str()) {
      if var.length() == 0
        || matches!(
          var.ty(),
          fstapi::var_type::GEN_STRING
            | fstapi::var_type::VCD_REAL
            | fstapi::var_type::VCD_REAL_PARAMETER
            | fstapi::var_type::VCD_REALTIME
            | fstapi::var_type::SV_SHORTREAL
            | fstapi::var_type::VCD_PORT
        )
      {
        return Err(format!("binding {path} is not fixed-width logic").into());
      }
      for &i in indices {
        widths[i] = var.length();
        found[i] = true;
      }
      mapping
        .entry(var.handle())
        .or_insert_with(Vec::new)
        .extend(indices.iter().copied());
    }
  }
  if found.iter().any(|v| !v) {
    return Err(
      format!(
        "missing bound signals: {}",
        names
          .iter()
          .enumerate()
          .filter(|(i, _)| !found[*i])
          .map(|(_, n)| n.as_str())
          .collect::<Vec<_>>()
          .join(", ")
      )
      .into(),
    );
  }
  let tick_index = names.len();
  let sample_index = tick_index + 1;
  let schema: HashMap<_, _> = names
    .iter()
    .enumerate()
    .map(|(i, name)| (name.clone(), (i, widths[i])))
    .chain([
      ("tick".into(), (tick_index, 64)),
      ("sample_index".into(), (sample_index, 64)),
    ])
    .collect();
  Ok(ResolvedSignals {
    schema,
    mapping,
    tick_index,
    sample_index,
  })
}
