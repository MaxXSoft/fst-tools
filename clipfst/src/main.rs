use clap::{Parser, ValueEnum};
use fstapi::{writer_pack_type, ScopeType, WriterPackType};
use fstapi::{Error, Handle, Hier, Reader, Result, Scope, Writer};
use regex::Regex;
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
    process::exit(1)
  }};
}

macro_rules! try_or_exit {
  ($r:expr, $e:ident, $($t:tt)*) => {
    match $r {
      Ok(v) => v,
      Err($e) => eprintln_exit!($($t)*),
    }
  };
  ($r:expr, _, $($t:tt)*) => {
    match $r {
      Ok(v) => v,
      Err(_) => eprintln_exit!($($t)*),
    }
  };
}

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

  // Build hierarchies for output FST file and
  // update signal masks for reader.
  let handles = build_output_hiers(&mut reader, &mut writer, signal_re, cli.strip_attrs)?;
  if handles.len() < (reader.var_count() - reader.alias_count()) as usize {
    reader.clear_mask_all();
    for handle in handles.keys() {
      reader.set_mask(*handle);
    }
  } else {
    reader.set_mask_all();
  }

  // Write value change data.
  let mut last_time = start;
  reader.for_each_block(|time, handle, value, var_len| {
    // Write time change.
    if time != last_time {
      let ret = writer.emit_time_change(time - start);
      try_or_exit!(ret, _, "Failed to write time change!");
      last_time = time;
    }
    // Write value change.
    let ret = if var_len {
      writer.emit_var_len_value_change(handles[&handle], value)
    } else {
      writer.emit_value_change(handles[&handle], value)
    };
    try_or_exit!(ret, _, "Failed to write value change!");
  })
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

struct ScopeStorage {
  ty: ScopeType,
  name: String,
  component: String,
  visited: bool,
}

impl TryFrom<Scope<'_>> for ScopeStorage {
  type Error = Error;

  fn try_from(s: Scope) -> Result<Self> {
    Ok(Self {
      ty: s.ty(),
      name: s.name()?.into(),
      component: s.component()?.into(),
      visited: false,
    })
  }
}

fn build_output_hiers(
  reader: &mut Reader,
  writer: &mut Writer,
  re: Option<Regex>,
  strip_attrs: bool,
) -> Result<HashMap<Handle, Handle>> {
  let mut scopes = Vec::new();
  let mut handles = HashMap::new();
  // Iterate over hierarchies of the input waveform.
  for hier in reader.hiers() {
    match hier {
      // If need to match signals, just store the scope.
      Hier::Scope(s) if re.is_some() => scopes.push(ScopeStorage::try_from(s)?),
      // Otherwise, write the current scope to the output.
      Hier::Scope(s) => writer.set_scope(s.ty(), s.name()?, s.component()?)?,

      // If no need to match signals, or there is a visited scope storage
      // (which means there are matching signals in this scope)
      // write the upscope to the output. Otherwise nothing to do.
      Hier::Upscope if re.is_none() || matches!(scopes.pop(), Some(s) if s.visited) => {
        writer.set_upscope()
      }

      Hier::Var(v) => {
        let name = v.name()?;
        // If need to match signals, check if the current signal matches.
        if let Some(re) = &re {
          if !re.is_match(name) {
            continue;
          }
          // Visit all unvisited scopes and write them to file.
          for s in scopes.iter_mut().filter(|s| !s.visited) {
            s.visited = true;
            writer.set_scope(s.ty, &s.name, &s.component)?;
          }
        }
        // Write the current variable to the output.
        let handle = writer.create_var(
          v.ty(),
          v.direction(),
          v.length(),
          name,
          handles.get(&v.handle()).copied(),
        )?;
        // Update mappings between input handles and output handles.
        handles.insert(v.handle(), handle);
      }

      // Write attributes only when `strip_attrs` is `false`.
      Hier::AttrBegin(a) if !strip_attrs => {
        writer.set_attr_begin(a.ty(), a.subtype() as i32, a.name()?, a.arg())?
      }
      Hier::AttrEnd if !strip_attrs => writer.set_attr_end(),
      _ => {}
    }
  }
  Ok(handles)
}
