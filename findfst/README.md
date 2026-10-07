# findfst

Find callback values in selected FST signals. Human-readable text remains the
default: `#TIME NAME VALUE`, or one name per result with `--names-only`.
Without `--all-matches`, the first matching callback is returned **per physical
handle**, not the first match globally. Aliases never duplicate results. Existing
binary, hexadecimal, and byte-regex matching semantics are preserved.

```sh
findfst trace.fst 107ae --hex --signals 'retire_pc'
findfst trace.fst '^1$' --regex --signals 'valid$' \
  --start 1000 --end 2000 --format json --max-rows 100 --max-bytes 65536
findfst trace.fst '.*' --regex --all-matches --json \
  --max-callbacks 100000 --max-duration-ms 500
```

`--format text` is explicit text mode. `--format json`
produces **JSON Lines**, one complete object per line; `jsonl` is an alias and
`--json` is a convenience spelling. `--json` and `--format` cannot be combined.
`--names-only` is supported only in text mode.

`-S` / `--signals` is an unanchored, case-sensitive Rust regular expression against
full names, including any declaration range suffix such as `pc [31:0]`. With a
signal filter, the displayed name is the canonical name if that name matches,
otherwise the first matching alias in hierarchy order. A selection matching no
signals remains a successful empty search.

## Window and match semantics

`--start` and `--end` are inclusive raw FST timestamps, defaulting to the file's
trace bounds. Both must be inside those bounds, and start must not exceed end.
A point query is allowed. Out-of-window callbacks neither produce matches nor
consume the first-match slot of a handle.

The search matches **callback observations**, which can include block-frame
snapshots and repeated values. It does not imply a signal transition. It does
not synthesize a match for a value carried into the requested window from an
earlier timestamp; use `queryfst` for explicit initial observations. The
backend skips whole blocks, so callbacks outside the exact window can still be
decoded. `findfst` applies the exact bounds before matching.

Default binary and hexadecimal exact matching zero-extend shorter values for
comparison. `--regex` matches callback bytes. With `--hex --regex`, only values
entirely composed of `0` and `1` participate; `x`/`z` and nonbinary values are
skipped without consuming a handle's first-match slot. Equal-time ordering is
libfst callback order and does not promise HDL delta-cycle order.

## JSON Lines schema version 1

The stdout stream contains:

1. `header`: `schema: "findfst"`, `schema_version: 1`, input path, trace bounds,
   inclusive requested bounds, timescale exponent, timezero, matching mode,
   pattern, interpretation, and configured limits.
2. `signal`: physical handle, selected display `name`, `canonical_name`, all
   `aliases`, FST declaration `width`, numeric `var_type`, and value `encoding`.
   These records are sorted by handle. Byte budgets can omit metadata records.
3. `dump_activity`: the file's recording-enable changes in original order.
   This metadata is not itself filtered by the requested time window. Disabled
   recording does not mean that signal values remained constant.
4. `match`: `sequence`, `time`, `handle`, selected `name`, `width`, `var_type`,
   `encoding`, and `value`. The record includes enough information to interpret
   its value even if that signal's separate metadata was omitted.
5. `summary`: execution status, output truncation, observed and exact counts,
   omitted metadata, and total stdout byte count.

Times, timezero, counters, sequence numbers, and limits are decimal strings.
Handles, widths, enum values, schema version, selected-handle count, and the
timescale exponent are JSON integers. Handles are bounded by unsigned 32-bit
FST handle numbers. One raw tick represents `10 ** timescale_exponent` seconds;
timezero is exposed separately and is never silently added to timestamps.

Match values use these encodings:

| Encoding | Representation |
| --- | --- |
| `bits` | Logic-character string, preserving width, leading zeroes, and `x`/`z` |
| `bytes_hex` | Lowercase hex for arbitrary variable-length bytes, including invalid UTF-8 and NUL |
| `evcd` | Callback text containing values and drive strengths |
| `real_decimal` | libfst's existing decimal real-value callback text |

Real matching retains the original tool's decimal callback behavior in both
formats. That formatting can round a double; it does **not** preserve original
IEEE-754 bits. Use `queryfst` when exact real bits are needed. No lossy UTF-8
replacement is performed; an unexpected non-UTF-8 callback uses `bytes_hex`.

`summary.status` is `complete` or `partial`, and `execution_complete` indicates
whether traversal finished normally. `output_truncated` is independent: a
complete search can have truncated output. `observed_matches`,
`emitted_matches`, and `observed_omitted_matches` count eligible records under
the selected first-per-handle/all-matches mode. `total_matches` and
`total_omitted_matches` are decimal strings only on complete execution; both are
`null` on partial execution. Zero observed matches in a partial search does not
establish that no matches exist. `metadata_complete` and
`omitted_metadata_records` describe separately omitted metadata.

`complete` and `reason` are aliases of `execution_complete` and `stop_reason`.
`unprocessed_input` is true on partial execution. `last_callback_time` gives the
last received callback timestamp; that timestamp's whole callback group may not
have been processed. `processed_through` identifies the last fully covered raw
timestamp, or `null` when no part of the requested interval is fully covered.
On complete execution it equals the requested end. All timestamps are decimal
strings. Crossing the exact end stops traversal successfully instead of spending
the remaining work budget on later callbacks in the enclosing block.

## Output and work budgets

All budgets are optional; the original unrestricted behavior remains the default.

- `--max-rows N` limits emitted match records. Metadata and the summary do not
  count as match rows. Zero emits no matches.
- `--max-bytes N` bounds **total serialized stdout bytes**, including JSON
  metadata, newlines, matches, and the summary. Its minimum is 4096 in both
  output formats. JSON reserves 2048 bytes for the summary. An oversized header
  is rejected before output; oversized metadata records are omitted whole.
  Once a match cannot fit, subsequent matches are also omitted, preserving an
  emitted prefix. Text mode likewise emits only whole records, including raw
  non-UTF-8 bytes when present.
- Row/byte limits alone do not stop decoding. The scan continues to calculate
  exact omitted-match totals, unless a separate work budget stops it.
- `--max-callbacks N` cooperatively stops the backend traversal after at most
  N received callbacks, including pre-start/post-end callbacks from included
  blocks. Zero stops before traversal. Reaching the count reports `partial`
  conservatively, even if the last callback happened to be the file's last one.
- `--max-duration-ms N` starts immediately before value traversal and is checked
  at callback boundaries. Zero stops before traversal. It is not a hard wall
  deadline: opening the input, hierarchy/metadata work, initial block decoding,
  an individual callback, and intervals without callbacks cannot be interrupted
  by this check. Backend buffers are released through the controlled reader API.

Work-budget stops identify `callback_budget_exhausted` or `duration_budget_exhausted` in
`summary.stop_reason`. Text mode emits a concise stderr status when execution
is partial or output was truncated, retaining the original stdout record format.
Intentional partial/truncated results exit successfully; consumers must inspect
the summary, not merely the exit code.

These options do not bound RSS. The selection catalog and alias names occupy
memory; libfst allocates hierarchy and per-block decoding buffers. A single
large callback or serialized record can require substantial temporary memory.

## Errors and completion

In JSON mode, failures produce one versioned `error` JSON object on **stderr**,
with `code`, `message`, and `exit_code`. Codes are `invalid_arguments` (exit 2),
`input_error`, `output_error`, or `encoding_error` (exit 1). Parse errors also use
JSON when `--json` or a recognized JSON `--format` was explicitly requested.
Argument-validation failures now consistently use exit 2; older invalid value
patterns used exit 1. Help/version output remains ordinary CLI text.

Failure during decoding or output can leave a partial stdout stream. Check the
process exit status and the final summary before accepting it. Stderr errors are
outside the stdout byte budget. A success summary buffered before a final flush
failure does not override a nonzero process exit.

Run `cargo test -p findfst` for human compatibility, structured output, boundary,
alias, byte/row/work budget, and error regressions. Generated fixtures stay in
Cargo's ignored build directory.
