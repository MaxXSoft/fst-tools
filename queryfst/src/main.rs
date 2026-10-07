mod error;
mod output;
mod query;
mod sampled;
mod sql;

use clap::{ArgGroup, CommandFactory, Parser, ValueEnum, error::ErrorKind};
use error::{Error, Result};
use std::io::{self, BufWriter};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
  Text,
  #[value(alias = "jsonl")]
  Json,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Matches {
  First,
  Last,
  All,
}

/// Inspect FST events or query periodically sampled state with a bounded SQL subset.
#[derive(Parser)]
#[command(author, version, about)]
#[command(group(ArgGroup::new("selection").required(true).multiple(true).args(["signals", "signal", "sql", "sql_file"])))]
#[command(group(ArgGroup::new("sql_source").args(["sql", "sql_file"]).requires("period")))]
#[command(group(ArgGroup::new("sampling").multiple(true).requires("sql_source").args([
  "bindings", "bind", "period", "phase", "max_samples", "max_groups", "max_buffer_rows",
  "matches", "before", "after",
])))]
struct Cli {
  /// Input FST waveform file.
  input: PathBuf,
  /// Regular expression matching full hierarchical signal paths.
  #[arg(short = 'S', long, value_name = "REGEX", conflicts_with_all = ["sql", "sql_file"])]
  signals: Option<regex::Regex>,
  /// Exact hierarchical path; repeat to select multiple signals.
  #[arg(long, value_name = "PATH", conflicts_with_all = ["sql", "sql_file"])]
  signal: Vec<String>,
  /// Inclusive start timestamp in raw FST ticks; defaults to the trace start.
  #[arg(long)]
  start: Option<u64>,
  /// Inclusive end timestamp in raw FST ticks; defaults to the trace end.
  #[arg(long)]
  end: Option<u64>,
  /// Maximum data rows: events, scalar summaries, or SQL rows; excludes metadata/footer.
  /// Output may be truncated independently of scan completion.
  #[arg(long, alias = "limit", default_value_t = 10_000)]
  max_rows: u64,
  /// Emit scalar residency/transition summaries instead of event records.
  #[arg(long, conflicts_with_all = ["sql", "sql_file"])]
  summary: bool,
  /// Output presentation. JSON is a versioned stream of JSON Lines.
  #[arg(long, value_enum, conflicts_with = "json")]
  format: Option<Format>,
  /// Shortcut for --format json.
  #[arg(long)]
  json: bool,
  /// Total stdout byte budget including metadata and final summary; minimum 4096.
  #[arg(long)]
  max_bytes: Option<u64>,
  /// Maximum delivered value callbacks, including pre-window state reconstruction.
  #[arg(long)]
  max_callbacks: Option<u64>,
  /// Cooperative scan duration budget; block decompression is not preempted.
  #[arg(long)]
  max_duration_ms: Option<u64>,
  /// SELECT query over the virtual samples table.
  #[arg(long, conflicts_with = "sql_file")]
  sql: Option<String>,
  /// UTF-8 file containing one SELECT query.
  #[arg(long)]
  sql_file: Option<PathBuf>,
  /// JSON object mapping SQL identifiers to exact waveform paths.
  #[arg(long)]
  bindings: Option<PathBuf>,
  /// Bind a SQL identifier to an exact path: NAME=PATH; repeat as needed.
  #[arg(long)]
  bind: Vec<String>,
  /// Sample at ticks t satisfying t % period == phase, after all changes at t.
  #[arg(long)]
  period: Option<u64>,
  /// Absolute sampling phase, less than period.
  #[arg(long, default_value_t = 0)]
  phase: u64,
  /// Maximum samples evaluated, including rows rejected by WHERE.
  #[arg(long)]
  max_samples: Option<u64>,
  /// Maximum distinct GROUP BY keys; bounds aggregation state cardinality.
  #[arg(long, default_value_t = 100_000)]
  max_groups: usize,
  /// Maximum rows retained for ORDER BY and context; bounds row cardinality.
  #[arg(long, default_value_t = 100_000)]
  max_buffer_rows: usize,
  /// Select first, last, or all WHERE triggers before expanding context.
  #[arg(long, value_enum, default_value = "all")]
  matches: Matches,
  /// Include this many preceding samples around WHERE matches (projection only).
  #[arg(long, default_value_t = 0)]
  before: u64,
  /// Include this many following samples around WHERE matches (projection only).
  #[arg(long, default_value_t = 0)]
  after: u64,
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
        "schema": "queryfst",
        "schema_version": 2,
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
    eprintln!("queryfst: {message}");
  }
}

fn main() {
  let args: Vec<_> = std::env::args_os().collect();
  let requested_json = json_requested(&args);
  let cli = match Cli::try_parse_from(args) {
    Ok(cli) => cli,
    Err(error) if requested_json && error.use_stderr() => {
      report_error(requested_json, "invalid_arguments", &error.to_string(), 2);
      std::process::exit(2);
    }
    Err(error) => error.exit(),
  };
  let format = cli.output_format();
  if let Err(error) = try_main(cli) {
    report_error(
      format == Format::Json,
      error.code(),
      &error.to_string(),
      error.exit_code(),
    );
    std::process::exit(error.exit_code());
  }
}

fn try_main(cli: Cli) -> Result<()> {
  let stdout = io::stdout();
  let mut output = output::Output::new(
    BufWriter::new(stdout.lock()),
    cli.output_format(),
    cli.max_bytes,
  )?;
  if cli.sql.is_some() || cli.sql_file.is_some() {
    sampled::run(cli, &mut output)
  } else {
    query::run(cli, &mut output)
  }
}
