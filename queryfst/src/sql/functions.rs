//! Shared builtin identities, canonical spellings, aliases, and call arities.

use sqlparser::ast::ObjectName;

/// Reduction identities; compiled inputs and per-group state live in the IR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AggregateFunction {
  Count,
  Sum,
  Min,
  Max,
}

/// Scalar identities; evaluation is implemented alongside bound expressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScalarFunction {
  /// Evaluates arguments only through the first non-NULL result.
  Coalesce,
  /// Includes the is_known alias.
  Known,
  Abs,
  Bit,
  Hex,
}

impl ScalarFunction {
  pub(super) fn accepts_arity(self, count: usize) -> bool {
    match self {
      Self::Coalesce => count > 0,
      Self::Known | Self::Abs | Self::Hex => count == 1,
      Self::Bit => count == 2,
    }
  }
}

/// Stateless temporal identities, resolved before constructing execution history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TemporalFunction {
  Timeouts,
  Lag,
  Changed,
  Hold,
  RunLength,
  Runs,
}

impl TemporalFunction {
  /// Returns the accepted arity nearest to the supplied count for diagnostics.
  pub(super) fn expected_arity(self, count: usize) -> usize {
    match self {
      Self::Timeouts => count.clamp(3, 4),
      Self::Hold => 2,
      Self::Lag | Self::Changed | Self::RunLength | Self::Runs => 1,
    }
  }
}

/// Classification shared by normalization, grouping detection, and lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BuiltinFunction {
  Aggregate(AggregateFunction),
  Scalar(ScalarFunction),
  Temporal(TemporalFunction),
  /// Requires bound-column width information when lowered.
  Raw,
}

/// One supported operation and every spelling that resolves to it.
struct FunctionDescriptor {
  canonical_name: &'static str,
  aliases: &'static [&'static str],
  kind: BuiltinFunction,
}

// Keep the catalog in one place while giving already-canonical names a string
// match fast path. A linear scan here regresses repeated temporal-call planning.
macro_rules! builtin_catalog {
  ($($canonical:literal $(| $alias:literal)* => $kind:ident $(($variant:path))?),* $(,)?) => {
    const BUILTINS: &[FunctionDescriptor] = &[
      $(FunctionDescriptor {
        canonical_name: $canonical,
        aliases: &[$($alias),*],
        kind: BuiltinFunction::$kind$(($variant))?,
      }),*
    ];

    fn lookup(name: &str) -> Option<BuiltinFunction> {
      match name {
        $($canonical $(| $alias)* => Some(BuiltinFunction::$kind$(($variant))?),)*
        _ => None,
      }
    }

    fn aggregate_name(name: &str) -> bool {
      match name {
        $($canonical $(| $alias)* => matches!(
          BuiltinFunction::$kind$(($variant))?,
          BuiltinFunction::Aggregate(_),
        ),)*
        _ => false,
      }
    }

    impl BuiltinFunction {
      pub(super) fn canonical_name(self) -> &'static str {
        match self {
          $(BuiltinFunction::$kind$(($variant))? => $canonical,)*
        }
      }
    }

    fn alias_name(name: &str) -> Option<&'static str> {
      match name {
        $($($alias => Some($canonical),)*)*
        _ => None,
      }
    }
  };
}

builtin_catalog! {
  "count" => Aggregate(AggregateFunction::Count),
  "sum" => Aggregate(AggregateFunction::Sum),
  "min" => Aggregate(AggregateFunction::Min),
  "max" => Aggregate(AggregateFunction::Max),
  "coalesce" => Scalar(ScalarFunction::Coalesce),
  "known" | "is_known" => Scalar(ScalarFunction::Known),
  "abs" => Scalar(ScalarFunction::Abs),
  "bit" => Scalar(ScalarFunction::Bit),
  "hex" => Scalar(ScalarFunction::Hex),
  "raw" => Raw,
  "lag" => Temporal(TemporalFunction::Lag),
  "changed" => Temporal(TemporalFunction::Changed),
  "hold" => Temporal(TemporalFunction::Hold),
  "run_length" => Temporal(TemporalFunction::RunLength),
  "runs" => Temporal(TemporalFunction::Runs),
  "timeouts" => Temporal(TemporalFunction::Timeouts),
}

/// Only single, unquoted names participate in builtin resolution. Case folding
/// never applies to quoted or qualified names, identifiers, or literal values.
#[inline]
pub(super) fn resolve(name: &ObjectName) -> Option<BuiltinFunction> {
  let name = unquoted_name(name)?;
  lookup(name).or_else(|| lookup_folded(name).map(|descriptor| descriptor.kind))
}

/// Grouping detection only needs aggregate membership, avoiding full resolution
/// for the scalar and temporal calls visited while walking an expression tree.
pub(super) fn is_aggregate(name: &ObjectName) -> bool {
  let Some(name) = unquoted_name(name) else {
    return false;
  };
  aggregate_name(name)
    || lookup_folded(name)
      .is_some_and(|descriptor| matches!(descriptor.kind, BuiltinFunction::Aggregate(_)))
}

/// Most expressions are already canonical. Only aliases and names containing
/// uppercase letters need resolution during the normalization preflight walk.
pub(super) fn canonical_replacement(name: &ObjectName) -> Option<&'static str> {
  let name = unquoted_name(name)?;
  alias_name(name).or_else(|| lookup_folded(name).map(|d| d.canonical_name))
}

fn unquoted_name(name: &ObjectName) -> Option<&str> {
  let [name] = name.0.as_slice() else {
    return None;
  };
  name.quote_style.is_none().then_some(name.value.as_str())
}

fn lookup_folded(name: &str) -> Option<&'static FunctionDescriptor> {
  if !name.bytes().any(|b| b.is_ascii_uppercase()) {
    return None;
  }
  BUILTINS.iter().find(|builtin| {
    name.eq_ignore_ascii_case(builtin.canonical_name)
      || builtin
        .aliases
        .iter()
        .any(|alias| name.eq_ignore_ascii_case(alias))
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use sqlparser::ast::Ident;
  use std::collections::HashSet;

  #[test]
  fn spellings_are_unique_and_resolve_to_their_canonical_operation() {
    let mut spellings = HashSet::new();
    for descriptor in BUILTINS {
      for spelling in
        std::iter::once(descriptor.canonical_name).chain(descriptor.aliases.iter().copied())
      {
        assert!(spellings.insert(spelling.to_ascii_lowercase()));
        for value in [spelling.to_string(), spelling.to_ascii_uppercase()] {
          let name = ObjectName(vec![Ident::new(value)]);
          let resolved = resolve(&name).unwrap();
          assert_eq!(resolved, descriptor.kind);
          assert_eq!(resolved.canonical_name(), descriptor.canonical_name);
          assert_eq!(
            is_aggregate(&name),
            matches!(descriptor.kind, BuiltinFunction::Aggregate(_))
          );
          let replacement = canonical_replacement(&name);
          assert_eq!(
            replacement,
            (name.0[0].value != descriptor.canonical_name).then_some(descriptor.canonical_name)
          );
        }
        assert!(resolve(&ObjectName(vec![Ident::with_quote('"', spelling)])).is_none());
        assert!(
          resolve(&ObjectName(vec![
            Ident::new("samples"),
            Ident::new(spelling)
          ]))
          .is_none()
        );
      }
    }
    assert!(resolve(&ObjectName(vec![Ident::new("unsupported")])).is_none());
  }
}
