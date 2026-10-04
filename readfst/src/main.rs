mod attrs;
mod json;
mod metadata;
mod scopes;
mod section;
mod vars;

use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};
use fstapi::Reader;
use regex::Regex;
use section::Print;
use std::io::{self, Write};
use std::process;
use vars::VarSection;

/// Errors from waveform reading and writing the structured response.
enum Error {
  Fst(fstapi::Error),
  Json(serde_json::Error),
  Io(io::Error),
}

impl std::fmt::Display for Error {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Fst(error) => error.fmt(f),
      Self::Json(error) => error.fmt(f),
      Self::Io(error) => error.fmt(f),
    }
  }
}

impl From<fstapi::Error> for Error {
  fn from(error: fstapi::Error) -> Self {
    Self::Fst(error)
  }
}

impl From<serde_json::Error> for Error {
  fn from(error: serde_json::Error) -> Self {
    Self::Json(error)
  }
}

impl From<io::Error> for Error {
  fn from(error: io::Error) -> Self {
    Self::Io(error)
  }
}

/// Output encoding; table preserves the original human-readable interface.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
  Table,
  Json,
}

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

  /// Equivalent to: -m -v -s -A.
  #[arg(short, long)]
  all: bool,

  /// Display the metadata of the waveform.
  #[arg(short, long)]
  metadata: bool,

  /// Display all variables.
  #[arg(short, long)]
  vars: bool,

  /// Do not display aliases when displaying variable names.
  #[arg(long)]
  no_aliases: bool,

  /// Only display variable name.
  #[arg(long)]
  names_only: bool,

  /// Display all scopes.
  #[arg(short, long)]
  scopes: bool,

  /// Display all attributes.
  #[arg(short = 'A', long)]
  attrs: bool,

  /// Output encoding. JSON emits one versioned document.
  #[arg(long, value_enum, default_value = "table")]
  format: Format,

  /// Include only variables whose full names match this regular expression.
  #[arg(long, value_name = "REGEX")]
  signals: Option<Regex>,
}

fn main() {
  if let Err(e) = try_main() {
    eprintln!("Failed to read FST waveform: {e}!");
    process::exit(1);
  }
}

fn try_main() -> Result<(), Error> {
  // Parse command line arguments.
  let mut cli = Cli::parse();
  if cli.all {
    cli.metadata = true;
    cli.vars = true;
    cli.scopes = true;
    cli.attrs = true;
  }

  // Validate command line arguments.
  if !cli.metadata && !cli.vars && !cli.scopes && !cli.attrs {
    Cli::command()
      .error(
        ErrorKind::MissingRequiredArgument,
        "select --metadata, --vars, --scopes, --attrs, or --all",
      )
      .exit();
  }
  if cli.format == Format::Json && cli.names_only {
    Cli::command()
      .error(
        ErrorKind::ArgumentConflict,
        "--names-only requires --format table",
      )
      .exit();
  }
  if cli.signals.is_some() && !cli.vars {
    Cli::command()
      .error(
        ErrorKind::MissingRequiredArgument,
        "--signals requires --vars or --all",
      )
      .exit();
  }

  // Open the given FST file.
  let mut reader = Reader::open(&cli.file)?;

  if cli.format == Format::Json {
    let document = json::Document::new(&mut reader, &cli)?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    serde_json::to_writer(&mut out, &document)?;
    writeln!(out)?;
    out.flush()?;
    return Ok(());
  }

  // Generate sections.
  let mut secs: Vec<Box<dyn Print>> = Vec::new();
  if cli.metadata {
    secs.push(Box::new(metadata::Metadata::new(&reader)?));
  }
  if cli.vars {
    secs.push(match (cli.no_aliases, cli.names_only) {
      (false, false) => Box::new(vars::Variables::new(&mut reader, cli.signals.as_ref())?),
      (true, false) => Box::new(vars::NoAliasesVars::new(&mut reader, cli.signals.as_ref())?),
      (false, true) => Box::new(vars::NameOnlyVars::new(&mut reader, cli.signals.as_ref())?),
      (true, true) => Box::new(vars::NameOnlyNoAliasesVars::new(
        &mut reader,
        cli.signals.as_ref(),
      )?),
    });
  }
  if cli.scopes {
    secs.push(Box::new(scopes::Scopes::new(&mut reader)?));
  }
  if cli.attrs {
    secs.push(Box::new(attrs::Attrs::new(&mut reader)?));
  }

  // Print sections.
  secs.print();
  Ok(())
}
