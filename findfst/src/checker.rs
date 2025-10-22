use fstapi::{Handle, Reader, Result};
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Map for stroing handles and their corresponding variable names.
pub type VarMap = HashMap<Handle, String>;

/// Array for stroing variable names.
pub type VarArray = Box<[String]>;

/// Information of variable, contains [`VarMap`] and [`VarArray`].
pub enum VarInfo {
  Map(VarMap),
  Array(VarArray),
}

impl VarInfo {
  pub fn new(reader: &mut Reader, re: Option<Regex>) -> Result<Self> {
    if let Some(re) = re {
      // Collect matching variables.
      let mut vars = HashMap::new();
      for var in reader.vars() {
        let (name, var) = var?;
        let handle = var.handle();
        if re.is_match(&name) && (!var.is_alias() || !vars.contains_key(&handle)) {
          vars.insert(handle, name);
        }
      }
      Ok(Self::Map(vars))
    } else {
      // Collect all variables.
      Ok(Self::Array(
        reader
          .vars()
          .filter_map(|var| var.map(|(n, v)| (!v.is_alias()).then_some(n)).transpose())
          .collect::<Result<Box<_>>>()?,
      ))
    }
  }
}

/// Trait for checking variables and getting their names.
pub trait VarChecker<T> {
  fn new(vars: T) -> Self;
  fn num_vars(&self) -> usize;
  fn check(&mut self, handle: Handle) -> Option<&str>;
}

/// Checks if the given handle is in the variable map.
pub struct SparseChecker {
  vars: VarMap,
}

impl VarChecker<VarMap> for SparseChecker {
  fn new(vars: VarMap) -> Self {
    Self { vars }
  }

  fn num_vars(&self) -> usize {
    self.vars.len()
  }

  fn check(&mut self, handle: Handle) -> Option<&str> {
    self.vars.get(&handle).map(|s| s.as_str())
  }
}

/// Checks if the given handle is in the variable array.
pub struct DenseChecker {
  vars: VarArray,
}

impl VarChecker<VarArray> for DenseChecker {
  fn new(vars: VarArray) -> Self {
    Self { vars }
  }

  fn num_vars(&self) -> usize {
    self.vars.len()
  }

  fn check(&mut self, handle: Handle) -> Option<&str> {
    self
      .vars
      .get(u32::from(handle) as usize - 1)
      .map(|s| s.as_str())
  }
}

/// Trait for checking if the given handle is visited only once.
pub trait OnceVisitor {
  fn new(num_vars: usize) -> Self;

  /// Visits the given handle, returns `true` on the first visit.
  fn visit(&mut self, handle: Handle) -> bool;
}

/// Visitor with a sparse set inside.
pub struct SparseVisitor {
  visited: HashSet<Handle>,
}

impl OnceVisitor for SparseVisitor {
  fn new(_: usize) -> Self {
    Self {
      visited: HashSet::new(),
    }
  }

  fn visit(&mut self, handle: Handle) -> bool {
    self.visited.insert(handle)
  }
}

/// Visitor with a dense set inside.
pub struct DenseVisitor {
  visited: Box<[u8]>,
}

impl OnceVisitor for DenseVisitor {
  fn new(num_vars: usize) -> Self {
    Self {
      visited: vec![0; num_vars.div_ceil(8)].into(),
    }
  }

  fn visit(&mut self, handle: Handle) -> bool {
    let index = u32::from(handle) as usize - 1;
    let byte = &mut self.visited[index / 8];
    let bit = 1 << (index % 8);
    let ret = (*byte & bit) == 0;
    *byte |= bit;
    ret
  }
}

/// Checks the given handle for only once.
pub struct OnceChecker<V, C> {
  visitor: V,
  checker: C,
}

impl<T, V, C> VarChecker<T> for OnceChecker<V, C>
where
  V: OnceVisitor,
  C: VarChecker<T>,
{
  fn new(vars: T) -> Self {
    let checker = C::new(vars);
    Self {
      visitor: V::new(checker.num_vars()),
      checker,
    }
  }

  fn check(&mut self, handle: Handle) -> Option<&str> {
    if self.visitor.visit(handle) {
      self.checker.check(handle)
    } else {
      None
    }
  }

  fn num_vars(&self) -> usize {
    self.checker.num_vars()
  }
}

/// Checks if the given handle is in the variable map for only once.
pub type SparseOnceChecker = OnceChecker<SparseVisitor, SparseChecker>;

/// Checks if the given handle is in the variable array for only once.
pub type DenseOnceChecker = OnceChecker<DenseVisitor, DenseChecker>;
