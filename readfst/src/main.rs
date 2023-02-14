use clap::Parser;
use fstapi::{file_type, Hier, Reader, Result};
use std::collections::HashMap;
use std::process;

#[derive(Parser)]
#[command(
  author,
  version,
  about,
  help_template(
    r#"
{before-help}{name} {version}, {author-with-newline}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}"#
  )
)]
struct Cli {
  /// FST waveform file.
  file: String,

  /// Equivalent to: -m -n.
  #[arg(short, long)]
  all: bool,

  /// Display the metadata of the waveform.
  #[arg(short, long)]
  metadata: bool,

  /// Display all variable names.
  #[arg(short = 'n', long)]
  var_names: bool,

  /// Do not display aliases when displaying variable names.
  #[arg(long)]
  no_aliases: bool,
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
    cli.var_names = true;
  }

  // Validate command line arguments.
  if !cli.metadata && !cli.var_names {
    eprintln!("Invalid command line arguments, try `-a`.");
    process::exit(1);
  }

  // Open the given FST file.
  let mut reader = Reader::open(cli.file)?;

  // Print metadata.
  if cli.metadata {
    print_metadata(&reader)?;
  }

  // Print variable names.
  if cli.var_names {
    println!();
    print_var_names(&mut reader, cli.no_aliases)?;
  }
  Ok(())
}

fn print_metadata(reader: &Reader) -> Result<()> {
  // Get file type.
  let file_type = match reader.file_type() {
    file_type::VERILOG => "Verilog",
    file_type::VHDL => "VHDL",
    file_type::VERILOG_VHDL => "Verilog/VHDL",
    _ => "Unknown",
  };
  // Get timescale.
  let timescale = match reader.timescale_str() {
    Some(t) => t,
    None => "Unknown",
  };
  // Print metadata.
  println!("FST metadata:");
  println!("  Date: {}", reader.date()?.trim());
  println!("  Version: {}", reader.version()?.trim());
  println!("  File type: {}", file_type);
  println!("  Timescale: {}", timescale);
  println!("  Timezero: {}", reader.timezero());
  println!("  Start time: {}", reader.start_time());
  println!("  End time: {}", reader.end_time());
  println!("  Number of scopes: {}", reader.scope_count());
  println!("  Number of variables: {}", reader.var_count());
  println!("  Number of aliases: {}", reader.alias_count());
  Ok(())
}

fn print_var_names(reader: &mut Reader, no_aliases: bool) -> Result<()> {
  println!("Variables:");
  let mut scopes = Vec::new();
  let mut vars = HashMap::new();
  // Iterate over hierarchies.
  for hier in reader.hiers() {
    match hier {
      Hier::Scope(s) => {
        let name = s.name()?;
        // Record scope name.
        match scopes.last() {
          Some(last) => scopes.push(format!("{last}.{name}")),
          None => scopes.push(name.to_string()),
        }
      }
      Hier::Upscope => {
        scopes.pop();
      }
      Hier::Var(v) if !no_aliases || !v.is_alias() => {
        // Get full variable name.
        let name = v.name()?;
        let name = match scopes.last() {
          Some(last) => format!("{last}.{name}"),
          None => name.to_string(),
        };
        // Print variable name.
        print!("  {name}");
        if v.is_alias() {
          print!(" (alias of {})", vars[&v.handle()]);
        }
        println!();
        // Update handle-name map.
        if !no_aliases && !v.is_alias() {
          vars.insert(v.handle(), name);
        }
      }
      _ => {}
    }
  }
  Ok(())
}
