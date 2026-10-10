//! CLI options and result records for sampled SQL queries.

use crate::sql::value::{Cell, Column};
use crate::{Cli, Error, Matches, Result, output::Output, sql};
use fstapi::Reader;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;

fn options(cli: &Cli, reader: &Reader) -> Result<sql::Options> {
  let mut bindings: BTreeMap<String, String> = if let Some(path) = &cli.bindings {
    serde_json::from_reader(std::fs::File::open(path)?)?
  } else {
    BTreeMap::new()
  };
  for binding in &cli.bind {
    let (name, path) = binding
      .split_once('=')
      .ok_or_else(|| Error::Arguments(format!("invalid --bind {binding:?}: expected NAME=PATH")))?;
    if name.is_empty() || path.is_empty() {
      return Err(Error::Arguments(format!(
        "invalid --bind {binding:?}: NAME and PATH must be nonempty"
      )));
    }
    if bindings.insert(name.into(), path.into()).is_some() {
      return Err(Error::Arguments(format!(
        "duplicate binding {name:?} in --bind {binding:?}"
      )));
    }
  }
  let sql = match (&cli.sql, &cli.sql_file) {
    (Some(sql), None) => sql.clone(),
    (None, Some(path)) => std::fs::read_to_string(path)?,
    _ => return Err("supply exactly one of --sql and --sql-file".into()),
  };
  let period = cli
    .period
    .ok_or("SQL sampling requires an explicit --period")?;
  Ok(sql::Options {
    sql,
    bindings,
    start: cli.start.unwrap_or(reader.start_time()),
    end: cli.end.unwrap_or(reader.end_time()),
    sampling: sql::PeriodicSampling {
      period,
      phase: cli.phase,
    },
    max_callbacks: cli.max_callbacks,
    max_samples: cli.max_samples,
    max_groups: Some(cli.max_groups),
    max_buffer_rows: Some(cli.max_buffer_rows),
    max_duration_ms: cli.max_duration_ms,
    matches: match cli.matches {
      Matches::First => sql::MatchMode::First,
      Matches::Last => sql::MatchMode::Last,
      Matches::All => sql::MatchMode::All,
    },
    context_before: cli.before,
    context_after: cli.after,
  })
}

/// Capture the header before traversal borrows the reader mutably.
fn header(reader: &Reader, options: &sql::Options) -> serde_json::Value {
  json!({
    "type": "header",
    "schema": "queryfst",
    "schema_version": 2,
    "mode": "samples",
    "start": options.start.to_string(),
    "end": options.end.to_string(),
    "trace_start": reader.start_time().to_string(),
    "trace_end": reader.end_time().to_string(),
    "timescale_exponent": reader.timescale(),
    "timezero": reader.timezero().to_string(),
    "period": options.sampling.period.to_string(),
    "phase": options.sampling.phase.to_string(),
    "sample_semantics": "after_complete_timestamp",
    "interval": "inclusive",
    "numeric_encoding": "decimal_string",
    "unknown_numeric": "null",
    "bindings": options.bindings,
    "sql": options.sql,
  })
}

fn write_header(
  output: &mut Output<impl Write>,
  header: &serde_json::Value,
  columns: &[Column],
) -> Result<bool> {
  if !output.record(header)? {
    return Ok(false);
  }
  output.record(&json!({
    "type": "columns",
    "columns": columns.iter().map(|c| &c.name).collect::<Vec<_>>(),
  }))
}

pub(crate) fn run(cli: Cli, output: &mut Output<impl Write>) -> Result<()> {
  let mut reader = Reader::open(&cli.input)?;
  let options = options(&cli, &reader)?;
  let header = header(&reader, &options);
  let mut header_written = false;
  let mut rows = 0;
  let mut row_limit = false;
  let report = sql::execute(&mut reader, &options, |columns, values| {
    if !header_written {
      header_written = true;
      if !write_header(output, &header, columns)? {
        return Ok(false);
      }
    }
    if rows >= cli.max_rows {
      row_limit = true;
      return Ok(false);
    }
    if !output.record(&json!({
      "type": "row",
      "values": values.iter().map(Cell::to_json).collect::<Vec<_>>(),
    }))? {
      return Ok(false);
    }
    rows += 1;
    Ok(true)
  })?;
  if !header_written {
    write_header(output, &header, &report.columns)?;
  }
  finish(output, &report, rows, row_limit)
}

fn finish(
  output: &mut Output<impl Write>,
  report: &sql::Report,
  rows: u64,
  row_limit: bool,
) -> Result<()> {
  let output_reason = if row_limit {
    Some("row_budget_exhausted")
  } else if output.truncated {
    Some("byte_budget_exhausted")
  } else {
    None
  };
  let reason = match report.stop_reason {
    Some(sql::StopReason::OutputLimit) | None => output_reason,
    reason => reason.map(sql::StopReason::as_str),
  };
  output.finish(json!({
    "type": "summary",
    "complete": report.complete,
    "status": if report.complete {"complete"} else {"partial"},
    "reason": reason,
    "output_reason": output_reason,
    "decoded_callbacks": report.decoded_callbacks.to_string(),
    "sampled_rows": report.sampled_rows.to_string(),
    "matched_rows": report.matched_rows.to_string(),
    "emitted_rows": rows.to_string(),
    "output_truncated": report.output_truncated || row_limit,
    "processed_through": report.processed_through.map(|n| n.to_string()),
    // A conservative query-level guarantee, not per-group accumulator coverage.
    "aggregate_final": report.complete,
    "scan_complete": report.scan_complete,
    "unprocessed_input": !report.scan_complete,
    "pending_requests": report.pending_requests.to_string(),
    "unresolved_due_to_unknown": report.unresolved_due_to_unknown.to_string(),
    "unmatched_responses": report.unmatched_responses.to_string(),
    "context_after_pending": report.context_after_pending.to_string(),
    "context_before_clipped": report.context_before_clipped,
  }))?;
  Ok(())
}
