mod checker;
mod find;
mod matcher;
mod output;
mod printer;

use checker::VarInfo;
use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};
use find::{MatchInfo, find_value};
use fstapi::Reader;
use std::io::Write;
use std::{fmt, io, process};

enum Error {
  Fst(fstapi::Error),
  Output(io::Error),
  Json(serde_json::Error),
  Arguments(String),
  Internal(&'static str),
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Fst(e) => e.fmt(f),
      Self::Output(e) => write!(f, "failed to write output: {e}"),
      Self::Json(e) => write!(f, "failed to encode output: {e}"),
      Self::Arguments(e) => e.fmt(f),
      Self::Internal(e) => f.write_str(e),
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

impl From<serde_json::Error> for Error {
  fn from(value: serde_json::Error) -> Self {
    Self::Json(value)
  }
}

impl Error {
  fn code(&self) -> &'static str {
    match self {
      Self::Fst(_) => "input_error",
      Self::Output(_) => "output_error",
      Self::Json(_) => "encoding_error",
      Self::Arguments(_) => "invalid_arguments",
      Self::Internal(_) => "internal_error",
    }
  }

  fn exit_code(&self) -> i32 {
    if matches!(self, Self::Arguments(_)) {
      2
    } else {
      1
    }
  }
}

/// Text remains the default; JSON is a stream of newline-delimited records.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
  Text,
  #[value(alias = "jsonl")]
  Json,
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
  /// Input FST waveform file.
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

  /// Regular expression matching full hierarchical signal paths.
  #[arg(short = 'S', long, value_name = "REGEX")]
  signals: Option<regex::Regex>,

  /// Print only signal names to stdout.
  #[arg(short, long)]
  names_only: bool,

  /// Output presentation. JSON is a versioned stream of JSON Lines.
  #[arg(long, value_enum, conflicts_with = "json")]
  format: Option<Format>,

  /// Shortcut for --format json.
  #[arg(long)]
  json: bool,

  /// Inclusive start timestamp in raw FST ticks; defaults to the trace start.
  #[arg(long)]
  start: Option<u64>,

  /// Inclusive end timestamp in raw FST ticks; defaults to the trace end.
  #[arg(long)]
  end: Option<u64>,

  /// Maximum match records to emit; counts continue unless a work budget stops decoding.
  #[arg(long)]
  max_rows: Option<u64>,

  /// Total stdout byte budget including metadata and final summary; minimum 4096.
  #[arg(long)]
  max_bytes: Option<u64>,

  /// Stop after processing this many callbacks, including callbacks outside the exact window.
  #[arg(long)]
  max_callbacks: Option<u64>,

  /// Cooperative scan duration budget; block decompression is not preempted.
  #[arg(long)]
  max_duration_ms: Option<u64>,
}

impl Cli {
  fn output_format(&self) -> Format {
    if self.json {
      Format::Json
    } else {
      self.format.unwrap_or(Format::Text)
    }
  }
}

/// Detect structured diagnostics even when Clap rejects the arguments.
fn json_requested(args: &[std::ffi::OsString]) -> bool {
  let mut args = args.iter().skip(1);
  while let Some(arg) = args.next() {
    if arg == "--" {
      break;
    }
    if arg == "--json" {
      return true;
    }
    if arg == "--format" {
      if args
        .next()
        .is_some_and(|value| value == "json" || value == "jsonl")
      {
        return true;
      }
    } else if arg == "--format=json" || arg == "--format=jsonl" {
      return true;
    }
  }
  false
}

/// Keep machine diagnostics on stderr, outside the result stream and byte budget.
fn report_error(json: bool, code: &str, message: &str, exit_code: i32) {
  if json {
    eprintln!(
      "{}",
      serde_json::json!({
        "schema": "findfst",
        "schema_version": 1,
        "type": "error",
        "code": code,
        "message": message,
        "exit_code": exit_code,
      })
    );
  } else {
    if exit_code == 2 {
      Cli::command()
        .error(ErrorKind::ValueValidation, message)
        .exit();
    }
    eprintln!("Failed to find in FST waveform: {message}!");
  }
}

fn main() {
  let args: Vec<_> = std::env::args_os().collect();
  let requested_json = json_requested(&args);
  let cli = match Cli::try_parse_from(args) {
    Ok(cli) => cli,
    Err(error) if requested_json && error.use_stderr() => {
      report_error(requested_json, "invalid_arguments", &error.to_string(), 2);
      process::exit(2);
    }
    Err(error) => error.exit(),
  };
  let json = cli.output_format() == Format::Json;
  if let Err(error) = try_main(cli) {
    report_error(json, error.code(), &error.to_string(), error.exit_code());
    process::exit(error.exit_code());
  }
}

fn try_main(cli: Cli) -> Result<()> {
  if cli.output_format() != Format::Text && cli.names_only {
    return Err(Error::Arguments("--names-only requires text output".into()));
  }
  if cli.max_bytes.is_some_and(|limit| limit < output::MIN_BYTES) {
    return Err(Error::Arguments(format!(
      "--max-bytes must be at least {}",
      output::MIN_BYTES
    )));
  }
  let match_info = MatchInfo::new(cli.value.clone(), cli.hex, cli.regex)
    .map_err(|error| Error::Arguments(error.to_string()))?;
  let mut reader = Reader::open(&cli.file)?;
  let start = cli.start.unwrap_or(reader.start_time());
  let end = cli.end.unwrap_or(reader.end_time());
  if start > end || start < reader.start_time() || end > reader.end_time() {
    return Err(Error::Arguments(format!(
      "range must satisfy {} <= start <= end <= {} (raw FST ticks)",
      reader.start_time(),
      reader.end_time()
    )));
  }
  let vars = VarInfo::new(&mut reader, cli.signals.clone())?;
  let catalog = if cli.output_format() == Format::Json {
    output::Catalog::new(&mut reader, &vars)?
  } else {
    output::Catalog::default()
  };
  match &vars {
    VarInfo::Map(vars) => {
      reader.clear_mask_all();
      for handle in vars.keys() {
        reader.set_mask(*handle);
      }
    }
    VarInfo::Array(_) => reader.set_mask_all(),
  }
  reader.set_time_range_limit(start, end);
  let stdout = io::stdout();
  let mut writer = io::BufWriter::new(stdout.lock());
  let mut output = output::Output::new(&mut writer, &cli, &reader, &catalog, start, end)?;
  let selected = vars.len();
  let scan = find_value(&mut reader, match_info, vars, &cli, start, end, &mut output)?;
  output.finish(&scan, selected)?;
  writer.flush()?;
  Ok(())
}
