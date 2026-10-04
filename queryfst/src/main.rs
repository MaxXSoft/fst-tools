mod query;

use clap::{ArgGroup, Parser};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Query selected FST waveform values as JSON Lines.
#[derive(Parser)]
#[command(author, version, about)]
#[command(group(ArgGroup::new("selection").required(true).multiple(true).args(["signals", "signal"])))]
struct Cli {
  /// Input FST waveform file.
  input: PathBuf,
  /// Regular expression matching full hierarchical paths (union with --signal).
  #[arg(short = 'S', long)]
  signals: Option<String>,
  /// Exact hierarchical path; repeat to select multiple signals.
  #[arg(long)]
  signal: Vec<String>,
  /// Inclusive start timestamp in raw FST ticks; defaults to the trace start.
  #[arg(long)]
  start: Option<u64>,
  /// Inclusive end timestamp in raw FST ticks; defaults to the trace end.
  #[arg(long)]
  end: Option<u64>,
  /// Maximum event records to emit; decoding continues to count omitted events.
  #[arg(long, default_value_t = 10_000)]
  limit: u64,
  /// Emit scalar residency/transition summaries instead of event records.
  #[arg(long)]
  summary: bool,
}

fn main() {
  let cli = Cli::parse();
  let stdout = io::stdout();
  let mut output = BufWriter::new(stdout.lock());
  let result = query::run(cli, &mut output).and_then(|()| Ok(output.flush()?));
  if let Err(error) = result {
    eprintln!("queryfst: {error}");
    std::process::exit(1);
  }
}
