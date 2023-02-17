mod attrs;
mod metadata;
mod scopes;
mod section;
mod vars;

use clap::Parser;
use fstapi::{Reader, Result};
use section::Print;
use std::process;
use vars::VarSection;

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

  /// Equivalent to: -m -v -A.
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
}

fn main() {
  if let Err(e) = try_main() {
    eprintln!("Failed to read FST waveform: {e}!");
    process::exit(1);
  }
}

fn try_main() -> Result<()> {
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
    eprintln!("Invalid command line arguments, try `-h`.");
    process::exit(1);
  }

  // Open the given FST file.
  let mut reader = Reader::open(cli.file)?;

  // Generate sections.
  let mut secs: Vec<Box<dyn Print>> = Vec::new();
  if cli.metadata {
    secs.push(Box::new(metadata::Metadata::new(&reader)?));
  }
  if cli.vars {
    secs.push(match (cli.no_aliases, cli.names_only) {
      (false, false) => Box::new(vars::Variables::new(&mut reader)?),
      (true, false) => Box::new(vars::NoAliasesVars::new(&mut reader)?),
      (false, true) => Box::new(vars::NameOnlyVars::new(&mut reader)?),
      (true, true) => Box::new(vars::NameOnlyNoAliasesVars::new(&mut reader)?),
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
