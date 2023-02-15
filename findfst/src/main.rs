mod checker;
mod find;
mod matcher;
mod printer;

use checker::{DenseChecker, DenseOnceChecker, SparseChecker, SparseOnceChecker};
use checker::{VarChecker, VarInfo};
use clap::Parser;
use find::MatchInfo;
use fstapi::{Handle, Reader, Result};
use matcher::{ExactMatcher, RegexHexMatcher, RegexMatcher, ValueMatcher};
use printer::{FullPrinter, NamePrinter, Printer};
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
  let match_info = try_or_exit!(MatchInfo::new(cli.value, cli.hex, cli.regex), e, "{e}");
  let signal_re = cli
    .signals
    .map(|s| try_or_exit!(regex::Regex::new(&s), e, "Invalid signal regex: {e}"));

  // Open the given FST file.
  let mut reader = Reader::open(cli.file)?;

  // Get variable information and update signal mask.
  let vars = VarInfo::new(&mut reader, signal_re)?;
  match &vars {
    VarInfo::Map(m) => {
      reader.clear_mask_all();
      for handle in m.keys() {
        reader.set_mask(*handle);
      }
    }
    VarInfo::Array(_) => reader.set_mask_all(),
  }

  // Iterate over blocks and find value.
  find_value(
    &mut reader,
    match_info,
    vars,
    cli.all_matches,
    cli.names_only,
  )
}

fn find_value(
  reader: &mut Reader,
  value_match: MatchInfo,
  vars: VarInfo,
  all_matches: bool,
  names_only: bool,
) -> Result<()> {
  match value_match {
    MatchInfo::Regex(re, true) => {
      find_value_m(reader, RegexMatcher::new(re), vars, all_matches, names_only)
    }
    MatchInfo::Regex(re, false) => find_value_m(
      reader,
      RegexHexMatcher::new(re),
      vars,
      all_matches,
      names_only,
    ),
    MatchInfo::Exact(e) => {
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
