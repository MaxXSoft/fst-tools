use clap::Parser;
use fstapi::{array_type, attr_type, enum_value_type, file_type, misc_type, pack_type};
use fstapi::{Attr, Hier, Reader, Result};
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

  /// Equivalent to: -m -n -A.
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
    cli.var_names = true;
    cli.attrs = true;
  }

  // Validate command line arguments.
  if !cli.metadata && !cli.var_names && !cli.attrs {
    eprintln!("Invalid command line arguments, try `-h`.");
    process::exit(1);
  }

  // Open the given FST file.
  let mut reader = Reader::open(cli.file)?;

  // Print metadata.
  let mut first = true;
  if cli.metadata {
    print_metadata(&reader)?;
    first = false;
  }

  // Print variable names.
  if cli.var_names {
    if !first {
      println!();
    } else {
      first = false;
    }
    print_var_names(&mut reader, cli.no_aliases)?;
  }

  // Print attributes.
  if cli.attrs {
    if !first {
      println!();
    }
    print_attrs(&mut reader)?;
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
  let mut vars = HashMap::new();
  // Iterate over variables.
  for var in reader.vars() {
    let (name, var) = var?;
    if no_aliases && var.is_alias() {
      continue;
    }
    // Print variable name.
    print!("  {name}");
    if var.is_alias() {
      print!(" (alias of {})", vars[&var.handle()]);
    }
    println!();
    // Update handle-name map.
    if !no_aliases && !var.is_alias() {
      vars.insert(var.handle(), name);
    }
  }
  Ok(())
}

fn print_attrs(reader: &mut Reader) -> Result<()> {
  println!("Attributes:");
  println!("  Num\tType\tSubType\tArg\tArgFromName\tName");
  let mut printed = 0;
  for hier in reader.hiers() {
    if let Hier::AttrBegin(attr) = hier {
      print!("  {printed}");
      print_attr(attr)?;
      printed += 1;
    }
  }
  if printed == 0 {
    println!("  None");
  }
  Ok(())
}

fn print_attr(attr: Attr) -> Result<()> {
  print!("\t");
  match attr.ty() {
    attr_type::MISC => {
      print!("Misc\t");
      match attr.subtype() {
        misc_type::COMMENT => print!("Comment"),
        misc_type::ENVVAR => print!("EnvVar"),
        misc_type::SUPVAR => print!("SupVar"),
        misc_type::PATHNAME => print!("PathName"),
        misc_type::SOURCESTEM => print!("SourceStem"),
        misc_type::SOURCEISTEM => print!("SourceIStem"),
        misc_type::VALUELIST => print!("ValueList"),
        misc_type::ENUMTABLE => print!("EnumTable"),
        misc_type::UNKNOWN => print!("Unknown"),
        _ => unreachable!(),
      }
    }
    attr_type::ARRAY => {
      print!("Array\t");
      match attr.subtype() {
        array_type::NONE => print!("None"),
        array_type::UNPACKED => print!("Unpacked"),
        array_type::PACKED => print!("Packed"),
        array_type::SPARSE => print!("Sparse"),
        _ => unreachable!(),
      }
    }
    attr_type::ENUM => {
      print!("Enum\t");
      match attr.subtype() {
        enum_value_type::SV_INTEGER => print!("SvInteger"),
        enum_value_type::SV_BIT => print!("SvBit"),
        enum_value_type::SV_LOGIC => print!("SvLogic"),
        enum_value_type::SV_INT => print!("SvInt"),
        enum_value_type::SV_SHORTINT => print!("SvShortint"),
        enum_value_type::SV_LONGINT => print!("SvLongint"),
        enum_value_type::SV_BYTE => print!("SvByte"),
        enum_value_type::SV_UNSIGNED_INTEGER => print!("SvUnsignedInteger"),
        enum_value_type::SV_UNSIGNED_BIT => print!("SvUnsignedBit"),
        enum_value_type::SV_UNSIGNED_LOGIC => print!("SvUnsignedLogic"),
        enum_value_type::SV_UNSIGNED_INT => print!("SvUnsignedInt"),
        enum_value_type::SV_UNSIGNED_SHORTINT => print!("SvUnsignedShortint"),
        enum_value_type::SV_UNSIGNED_LONGINT => print!("SvUnsignedLongint"),
        enum_value_type::SV_UNSIGNED_BYTE => print!("SvUnsignedByte"),
        enum_value_type::REG => print!("Reg"),
        enum_value_type::TIME => print!("Time"),
        _ => unreachable!(),
      }
    }
    attr_type::PACK => {
      print!("Pack\t");
      match attr.subtype() {
        pack_type::NONE => print!("None"),
        pack_type::UNPACKED => print!("Unpacked"),
        pack_type::PACKED => print!("Packed"),
        pack_type::TAGGED_PACKED => print!("TaggedPacked"),
        _ => unreachable!(),
      }
    }
    _ => unreachable!(),
  }
  println!(
    "\t{}\t{}\t{}",
    attr.arg(),
    attr.arg_from_name(),
    attr.name()?
  );
  Ok(())
}
