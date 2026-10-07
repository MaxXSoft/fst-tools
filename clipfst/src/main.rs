mod hiers;
mod vcd;

use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};
use fstapi::{Reader, Result, Writer, WriterPackType, writer_pack_type};
use vcd::VcdWriter;

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
  input: String,

  /// Output FST waveform file. Must be different from the input.
  output: String,

  /// Inclusive start timestamp in raw FST ticks; defaults to the trace start.
  #[arg(short, long)]
  start: Option<u64>,

  /// Inclusive end timestamp in raw FST ticks; defaults to the trace end.
  #[arg(short, long)]
  end: Option<u64>,

  /// Regular expression matching full hierarchical signal paths.
  #[arg(short = 'S', long, value_name = "REGEX")]
  signals: Option<regex::Regex>,

  /// Strip all attributes of the input waveform.
  #[arg(short = 't', long)]
  strip_attrs: bool,

  /// Do not use compressed hierarchy.
  #[arg(short, long)]
  no_comp_hier: bool,

  /// Specify pack type of the value change data.
  #[arg(short = 'P', long, value_enum, default_value_t = PackType::Lz4)]
  pack_type: PackType,

  /// Repack the entire waveform through gzip on close.
  #[arg(short, long)]
  repack: bool,

  /// Use parallel mode for output waveform writing.
  #[arg(short, long)]
  parallel: bool,
}

#[derive(Clone, ValueEnum)]
pub enum PackType {
  /// Pack value change data with LZ4.
  #[value(name = "4")]
  Lz4,
  /// Pack value change data with FastLZ.
  #[value(name = "f")]
  FastLz,
  /// Pack value change data with zlib.
  #[value(name = "z")]
  Zlib,
}

impl From<PackType> for WriterPackType {
  fn from(pt: PackType) -> Self {
    match pt {
      PackType::Lz4 => writer_pack_type::LZ4,
      PackType::FastLz => writer_pack_type::FASTLZ,
      PackType::Zlib => writer_pack_type::ZLIB,
    }
  }
}

macro_rules! eprintln_exit {
  ($($t:tt)*) => {{
    eprintln!($($t)*);
    std::process::exit(1)
  }};
}
pub(crate) use eprintln_exit;

macro_rules! try_or_exit {
  ($r:expr, $e:ident, $($t:tt)*) => {
    match $r {
      Ok(v) => v,
      Err($e) => $crate::eprintln_exit!($($t)*),
    }
  };
  ($r:expr, _, $($t:tt)*) => {
    match $r {
      Ok(v) => v,
      Err(_) => $crate::eprintln_exit!($($t)*),
    }
  };
}

fn main() {
  try_or_exit!(try_main(), e, "Failed to clip the FST waveform: {e}!");
}

fn try_main() -> Result<()> {
  // Parse command line arguments.
  let cli = Cli::parse();

  // Open the given FST file.
  let mut reader = Reader::open(&cli.input)?;

  // libfst unlinks an existing output before creating its writer. Reject
  // aliases of the input as well as identical path strings before that happens.
  match same_file::is_same_file(&cli.input, &cli.output) {
    Ok(true) => Cli::command()
      .error(
        ErrorKind::ArgumentConflict,
        "input and output must be different files",
      )
      .exit(),
    Ok(false) => {}
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
    Err(e) => eprintln_exit!("Failed to compare input and output files: {e}!"),
  }

  // Get and set start time and end time.
  let (start, end) = get_start_end(&reader, cli.start, cli.end);
  let timezero = try_or_exit!(
    i64::try_from(i128::from(reader.timezero()) + i128::from(start)),
    _,
    "Clipped timezero exceeds the signed 64-bit range: {} + {start}!",
    reader.timezero()
  );

  // Create the output FST file.
  let mut writer = Writer::create(cli.output, !cli.no_comp_hier)?
    .date(reader.date()?)?
    .version(reader.version()?)?
    .file_type(reader.file_type())
    .timescale(reader.timescale())
    .timezero(timezero)
    .pack_type(cli.pack_type.into())
    .repack_on_close(cli.repack)
    .parallel_mode(cli.parallel);

  // Build hierarchies for output FST file.
  let selection = hiers::build(&mut reader, &mut writer, cli.signals, cli.strip_attrs)?;

  // Update signal masks for reader.
  if selection.handles.len() < (reader.var_count() - reader.alias_count()) as usize {
    if selection.handles.is_empty() {
      eprintln_exit!("No matching signals!");
    }
    reader.clear_mask_all();
    for handle in selection.handles.keys() {
      reader.set_mask(*handle);
    }
  } else {
    reader.set_mask_all();
  }

  // Write value change data.
  VcdWriter::new(writer, start, end, selection).write(&mut reader)
}

fn get_start_end(reader: &Reader, start: Option<u64>, end: Option<u64>) -> (u64, u64) {
  macro_rules! get_time {
    ($time:expr, $prompt:expr, $default:expr) => {
      if let Some(time) = $time {
        if time < reader.start_time() || time > reader.end_time() {
          Cli::command()
            .error(
              ErrorKind::ValueValidation,
              format!(concat!("invalid ", $prompt, " time: {}"), time),
            )
            .exit();
        }
        time
      } else {
        $default
      }
    };
  }
  let start = get_time!(start, "start", reader.start_time());
  let end = get_time!(end, "end", reader.end_time());
  if start > end {
    Cli::command()
      .error(
        ErrorKind::ValueValidation,
        format!("invalid time range: {start}-{end}"),
      )
      .exit();
  }
  (start, end)
}
