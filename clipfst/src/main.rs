mod hiers;
mod vcd;

use clap::{Parser, ValueEnum};
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

  /// Output FST waveform file.
  output: String,

  /// Start time of the clip, default to the beginning.
  #[arg(short, long)]
  start: Option<u64>,

  /// End time of the clip, default to the ending.
  #[arg(short, long)]
  end: Option<u64>,

  /// Keep matching signals only, support regex.
  #[arg(short = 'S', long)]
  signals: Option<String>,

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
pub(crate) use try_or_exit;

fn main() {
  try_or_exit!(try_main(), e, "Failed to clip the FST waveform: {e}!");
}

fn try_main() -> Result<()> {
  // Parse command line arguments.
  let cli = Cli::parse();

  // Validate command line arguments.
  let signal_re = cli
    .signals
    .map(|s| try_or_exit!(regex::Regex::new(&s), e, "Invalid signal regex: {e}"));

  // Open the given FST file.
  let mut reader = Reader::open(cli.input)?;

  // Get and set start time and end time.
  let (start, end) = get_start_end(&reader, cli.start, cli.end);
  reader.set_time_range_limit(start, end);

  // Create the output FST file.
  let mut writer = Writer::create(cli.output, !cli.no_comp_hier)?
    .date(reader.date()?)?
    .version(reader.version()?)?
    .file_type(reader.file_type())
    .timescale(reader.timescale())
    .timezero(start as i64)
    .pack_type(cli.pack_type.into())
    .repack_on_close(cli.repack)
    .parallel_mode(cli.parallel);

  // Build hierarchies for output FST file.
  let handles = hiers::build(&mut reader, &mut writer, signal_re, cli.strip_attrs)?;

  // Update signal masks for reader.
  if handles.len() < (reader.var_count() - reader.alias_count()) as usize {
    if handles.is_empty() {
      eprintln_exit!("No matching signals!");
    }
    reader.clear_mask_all();
    for handle in handles.keys() {
      reader.set_mask(*handle);
    }
  } else {
    reader.set_mask_all();
  }

  // Write value change data.
  VcdWriter::new(writer, start, end, handles).write(&mut reader)
}

fn get_start_end(reader: &Reader, start: Option<u64>, end: Option<u64>) -> (u64, u64) {
  macro_rules! get_time {
    ($time:expr, $prompt:expr, $default:expr) => {
      if let Some(time) = $time {
        if time < reader.start_time() || time > reader.end_time() {
          eprintln_exit!(concat!("Invalid ", $prompt, " time: {}!"), time);
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
    eprintln_exit!("Invalid time range: {start}-{end}!");
  }
  (start, end)
}
