mod checker;
mod find;
mod matcher;
mod printer;

use checker::VarInfo;
use clap::Parser;
use find::{MatchInfo, find_value};
use fstapi::Reader;
use std::{fmt, io, process};

enum Error {
  Fst(fstapi::Error),
  Output(io::Error),
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Fst(e) => e.fmt(f),
      Self::Output(e) => write!(f, "failed to write output: {e}"),
    }
  }
}

impl From<fstapi::Error> for Error {
  fn from(value: fstapi::Error) -> Self {
    Self::Fst(value)
  }
}

impl From<io::Error> for Error {
  fn from(value: io::Error) -> Self {
    Self::Output(value)
  }
}

type Result<T> = std::result::Result<T, Error>;

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
  /// With --regex, values containing states other than 0 and 1 are skipped.
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

macro_rules! try_or_exit {
  ($r:expr, $e:ident, $($t:tt)*) => {
    match $r {
      Ok(v) => v,
      Err($e) => {
        eprintln!($($t)*);
        process::exit(1)
      }
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
