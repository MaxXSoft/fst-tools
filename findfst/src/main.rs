mod checker;
mod matcher;
mod printer;

use checker::{DenseChecker, DenseOnceChecker, SparseChecker, SparseOnceChecker};
use checker::{VarArray, VarChecker, VarMap};
use clap::Parser;
use fstapi::{Handle, Reader, Result};
use matcher::{ExactMatcher, RegexHexMatcher, RegexMatcher, ValueMatcher};
use printer::{FullPrinter, NamePrinter, Printer};
use std::collections::HashMap;
use std::process;

#[derive(Parser)]
#[command(
  author,
  version,
  about,
  help_template(
    r#"
{before-help}{name} {version} by {author-with-newline}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}"#
  )
)]
struct Cli {
  /// FST waveform file.
  file: String,

  /// The value to find, in binary format by default.
  value: String,

  /// Use lowercase hexadecimal format value instead of binary format.
  #[arg(short = 'x', long)]
  hex: bool,

  /// Find all matching values in a signal, not just the first match.
  #[arg(short, long)]
  all_matches: bool,

  /// Use regex to match values.
  #[arg(short, long)]
  regex: bool,

  /// Find value in matching signals only, support regex.
  #[arg(short, long)]
  signals: Option<String>,

  /// Print only signal names to stdout.
  #[arg(short, long)]
  names_only: bool,
}

macro_rules! eprintln_exit {
  ($($t:tt)*) => {{
    eprintln!($($t)*);
    process::exit(1)
  }};
}

macro_rules! try_or_exit {
  ($r:expr, $e:ident, $($t:tt)*) => {
    match $r {
      Ok(v) => v,
      Err($e) => eprintln_exit!($($t)*),
    }
  };
}

fn main() {
  try_or_exit!(try_main(), e, "Failed to find in FST waveform: {e}!");
}

fn try_main() -> Result<()> {
  // Parse command line arguments.
  let cli = Cli::parse();

  // Validate command line arguments.
  let value_match = ValueMatch::new(cli.value, cli.hex, cli.regex);
  let signal_re = cli
    .signals
    .map(|s| try_or_exit!(regex::Regex::new(&s), e, "Invalid signal regex: {e}"));

  // Open the given FST file.
  let mut reader = Reader::open(cli.file)?;

  // Update signal mask and get variable map.
  let vars = update_signal_mask(&mut reader, signal_re)?;

  // Iterate over blocks and find value.
  find_value(
    &mut reader,
    value_match,
    vars,
    cli.all_matches,
    cli.names_only,
  )
}

enum ValueMatch {
  Regex(regex::bytes::Regex, bool),
  Exact(Box<[u8]>),
}

impl ValueMatch {
  fn new(value: String, hex: bool, regex: bool) -> Self {
    if regex {
      let re = regex::bytes::Regex::new(&value);
      Self::Regex(try_or_exit!(re, e, "Invalid value regex: {e}"), hex)
    } else if hex {
      let mut s = Vec::new();
      for c in value.chars() {
        let digit = match c.to_digit(16) {
          Some(d) => d,
          _ => eprintln_exit!("Invalid hexadecimal value: {value}!"),
        };
        s.push(if (digit & 8) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 4) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 2) != 0 { b'1' } else { b'0' });
        s.push(if (digit & 1) != 0 { b'1' } else { b'0' });
      }
      Self::Exact(s.into())
    } else {
      if value.contains(|c: char| !c.is_digit(2)) {
        eprintln_exit!("Invalid binary value: {value}!");
      }
      Self::Exact(value.into_bytes().into())
    }
  }
}

enum VarInfo {
  Map(VarMap),
  Array(VarArray),
}

fn update_signal_mask(reader: &mut Reader, re: Option<regex::Regex>) -> Result<VarInfo> {
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
    // Update mask.
    reader.clear_mask_all();
    for handle in vars.keys() {
      reader.set_mask(*handle);
    }
    Ok(VarInfo::Map(vars))
  } else {
    // Update mask.
    reader.set_mask_all();
    // Collect all variables.
    Ok(VarInfo::Array(
      reader
        .vars()
        .filter_map(|var| var.map(|(n, v)| (!v.is_alias()).then_some(n)).transpose())
        .collect::<Result<Box<_>>>()?,
    ))
  }
}

fn find_value(
  reader: &mut Reader,
  value_match: ValueMatch,
  vars: VarInfo,
  all_matches: bool,
  names_only: bool,
) -> Result<()> {
  match value_match {
    ValueMatch::Regex(re, true) => {
      find_value_m(reader, RegexMatcher::new(re), vars, all_matches, names_only)
    }
    ValueMatch::Regex(re, false) => find_value_m(
      reader,
      RegexHexMatcher::new(re),
      vars,
      all_matches,
      names_only,
    ),
    ValueMatch::Exact(e) => {
      find_value_m(reader, ExactMatcher::new(e), vars, all_matches, names_only)
    }
  }
}

fn find_value_m<M>(
  reader: &mut Reader,
  value_matcher: M,
  vars: VarInfo,
  all_matches: bool,
  names_only: bool,
) -> Result<()>
where
  M: ValueMatcher,
{
  match (vars, all_matches) {
    (VarInfo::Map(vars), false) => {
      find_value_mc(reader, value_matcher, SparseChecker::new(vars), names_only)
    }
    (VarInfo::Map(vars), true) => find_value_mc(
      reader,
      value_matcher,
      SparseOnceChecker::new(vars),
      names_only,
    ),
    (VarInfo::Array(vars), false) => {
      find_value_mc(reader, value_matcher, DenseChecker::new(vars), names_only)
    }
    (VarInfo::Array(vars), true) => find_value_mc(
      reader,
      value_matcher,
      DenseOnceChecker::new(vars),
      names_only,
    ),
  }
}

fn find_value_mc<M, T, C>(
  reader: &mut Reader,
  value_matcher: M,
  var_checker: C,
  names_only: bool,
) -> Result<()>
where
  M: ValueMatcher,
  C: VarChecker<T> + Clone,
{
  if names_only {
    find_value_mcp(reader, value_matcher, var_checker, NamePrinter)
  } else {
    find_value_mcp(reader, value_matcher, var_checker, FullPrinter)
  }
}

fn find_value_mcp<M, T, C, P>(
  reader: &mut Reader,
  value_matcher: M,
  var_checker: C,
  printer: P,
) -> Result<()>
where
  M: ValueMatcher,
  C: VarChecker<T> + Clone,
  P: Printer,
{
  let mut var_checker = var_checker;
  let mut var_checker2 = var_checker.clone();
  let callback = |time, handle, value: &[u8]| {
    check_value(
      &value_matcher,
      &mut var_checker,
      &printer,
      time,
      handle,
      value,
    )
  };
  let callback2 = |time, handle, value: &[u8]| {
    check_value(
      &value_matcher,
      &mut var_checker2,
      &printer,
      time,
      handle,
      value,
    )
  };
  reader.for_each_block(callback, callback2)
}

fn check_value<M, T, C, P>(
  value_matcher: &M,
  var_checker: &mut C,
  printer: &P,
  time: u64,
  handle: Handle,
  value: &[u8],
) where
  M: ValueMatcher,
  C: VarChecker<T>,
  P: Printer,
{
  // Check if value matches.
  if value_matcher.is_match(value) {
    // Check the current variable and print.
    if let Some(name) = var_checker.check(handle) {
      printer.print(time, name, value);
    }
  }
}
