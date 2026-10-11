//! Bounded whole-record output shared by raw and sampled queries.

use crate::{Error, Format, Result};
use serde_json::{Value, json};
use std::io::{self, Write};

pub const FOOTER_RESERVE: u64 = 2048;

pub struct Output<W> {
  writer: W,
  format: Format,
  max_bytes: Option<u64>,
  pub bytes: u64,
  pub truncated: bool,
}

impl<W: Write> Output<W> {
  pub fn new(writer: W, format: Format, max_bytes: Option<u64>) -> Result<Self> {
    if max_bytes.is_some_and(|n| n < 4096) {
      return Err(Error::Arguments(
        "--max-bytes must be at least 4096 (includes a reserved final summary)".into(),
      ));
    }
    Ok(Self {
      writer,
      format,
      max_bytes,
      bytes: 0,
      truncated: false,
    })
  }

  pub fn record(&mut self, value: &Value) -> Result<bool> {
    if self.truncated {
      return Ok(false);
    }
    let bytes = self.encode(value)?;
    if self.max_bytes.is_some_and(|n| {
      self
        .bytes
        .saturating_add(bytes.len() as u64)
        .saturating_add(FOOTER_RESERVE)
        > n
    }) {
      self.truncated = true;
      return Ok(false);
    }
    self.write(&bytes)?;
    Ok(true)
  }

  pub fn finish(&mut self, mut value: Value) -> Result<()> {
    value["output_truncated"] =
      json!(self.truncated || value["output_truncated"].as_bool().unwrap_or(false));
    value["bytes_before_summary"] = self.bytes.to_string().into();
    if self.truncated {
      value["output_reason"] = "byte_budget_exhausted".into();
      if value.get("truncated").is_some() {
        value["truncated"] = true.into();
      }
    }
    let bytes = self.encode(&value)?;
    if self
      .max_bytes
      .is_some_and(|n| self.bytes.saturating_add(bytes.len() as u64) > n)
    {
      return Err(Error::Output(io::Error::other(
        "final summary exceeds reserved output budget",
      )));
    }
    self.write(&bytes)?;
    self.writer.flush().map_err(Error::Output)
  }

  fn write(&mut self, bytes: &[u8]) -> Result<()> {
    self.writer.write_all(bytes).map_err(Error::Output)?;
    self.bytes += bytes.len() as u64;
    Ok(())
  }

  fn encode(&self, value: &Value) -> Result<Vec<u8>> {
    if self.format == Format::Json {
      let mut bytes = serde_json::to_vec(value)?;
      bytes.push(b'\n');
      return Ok(bytes);
    }
    let text = |key: &str| display(&value[key]);
    let line = match value["type"].as_str().unwrap_or("") {
      "header" => format!(
        "FST query: {}..={} ticks ({})\n",
        text("start"),
        text("end"),
        text("mode")
      ),
      "signal" => format!(
        "  #{} {} [{} bits, {}]\n",
        text("handle"),
        text("path"),
        text("width"),
        text("encoding")
      ),
      "dump_activity" => format!(
        "  recording at {}: {}\n",
        text("time"),
        if value["active"] == true { "on" } else { "off" }
      ),
      "initial" => format!(
        "  initial #{} before {}: {}\n",
        text("handle"),
        text("time"),
        text("value")
      ),
      "event" => format!("{}\t#{}\t{}\n", text("time"), text("handle"), text("value")),
      "columns" => format!(
        "{}\n",
        value["columns"]
          .as_array()
          .unwrap()
          .iter()
          .map(display)
          .collect::<Vec<_>>()
          .join("\t")
      ),
      "row" => format!(
        "{}\n",
        value["values"]
          .as_array()
          .unwrap()
          .iter()
          .map(display)
          .collect::<Vec<_>>()
          .join("\t")
      ),
      "scalar_summary" => format!(
        "#{} residency={} transitions={}\n",
        text("handle"),
        text("residency_ticks"),
        text("value_transitions")
      ),
      "summary" => format!(
        "Query {}{}; callbacks={}, rows={}, output={}{}\n",
        if value["complete"] == true {
          "complete"
        } else {
          "partial"
        },
        value["reason"]
          .as_str()
          .map(|s| format!(" ({s})"))
          .unwrap_or_default(),
        text("decoded_callbacks"),
        value
          .get("emitted_rows")
          .map(display)
          .unwrap_or_else(|| text("emitted_events")),
        if value["output_truncated"] == true || value["truncated"] == true {
          "truncated"
        } else {
          "complete"
        },
        value
          .get("sampled_rows")
          .map(|n| format!(", samples={}", display(n)))
          .unwrap_or_default()
      ),
      _ => format!("{}\n", display(value)),
    };
    Ok(line.into_bytes())
  }
}

fn display(value: &Value) -> String {
  match value {
    Value::Null => "unavailable".into(),
    Value::String(s) => s
      .replace('\t', "\\t")
      .replace('\n', "\\n")
      .replace('\r', "\\r"),
    _ => value.to_string(),
  }
}
