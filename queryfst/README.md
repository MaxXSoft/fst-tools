# queryfst

Query selected FST signals as JSON Lines without converting the full waveform to
VCD or accumulating the event history in memory. This is a generic, read-only
query primitive for scripts and agents; it does not infer clock cycles,
instruction retirement, or hardware bottlenecks from signal names.

```sh
cargo build --release -p queryfst
target/release/queryfst trace.fst --signal top.core.retire_valid \
  --signal top.core.retire_pc --start 120000 --end 130000 --limit 1000
target/release/queryfst trace.fst --signals 'top\.core\.(stall|retire_valid)$' \
  --start 120000 --end 130000 --summary
```

At least one `--signal EXACT_PATH` or `--signals REGEX` is required. Repeated
exact paths and the regex form a union. Regex matching uses Rust's `regex` syntax;
anchor the expression with `^` and `$` when a whole-path match is needed. Every
exact path must exist. A selection matching no signals is an error. Aliases
select their physical handle; selecting several aliases never duplicates events.

## JSONL schema version 1

Every line is one JSON object with a `type` field. The stream contains:

1. `header`: schema name/version, requested interval, trace bounds, timescale,
   timezero, mode, output limit, ordering/initial-state semantics and scan start.
2. `signal`: physical `handle`, canonical `path`, all `aliases`, bit `width`,
   libfst numeric `var_type`, and value `encoding`. Signal metadata is ordered by
   handle. Names are UTF-8 strings; invalid UTF-8 hierarchy names are rejected.
3. `dump_activity`: all recorded dump-enable changes, retaining file order.
   Initial recording state is active before the first entry. Recording disabled
   does not mean the signals were constant.
4. `initial`: one value per selected handle at the requested start boundary.
5. `event`: at most `--limit` callback records, or `scalar_summary` records in
   summary mode.
6. `summary`: `complete: true`, selected handle count, decoded/matching callback
   counts, emitted/omitted event counts and `truncated`.

Times, timezero, durations, sequence numbers and potentially large counters are
**decimal strings**. Handles, widths, small schema/type identifiers and the
timescale exponent are JSON integers. Raw times are FST ticks; one tick is
`10 ** timescale_exponent` seconds. `timezero` is exposed as metadata and is not
silently added to callback timestamps.

`event` and `initial` records refer to the signal metadata by `handle`. Their
`value` is a string interpreted using `encoding`:

| Encoding | Value |
| --- | --- |
| `bits` | Raw logic characters, preserving widths, leading zeroes and `x`/`z` |
| `bytes_hex` | Lowercase hex of arbitrary variable-length bytes, including NUL and non-UTF-8 |
| `real_f64_le_hex` | Eight little-endian IEEE-754 bytes as hex, preserving exact FST double bits |
| `evcd` | Raw EVCD value and drive strengths, including their separators |

An unavailable initial value is JSON `null`. Real values use libfst's native
double callback; they are not rounded through a decimal representation. All
signals are represented losslessly within the FST reader's callback model.

## Boundaries, initial state and ordering

`--start` and `--end` are inclusive raw timestamps. Defaults are the trace
bounds. Reversed or out-of-trace bounds are rejected, rather than silently
clipped. A point query with equal bounds is allowed.

`initial` contains the latest callback value **strictly before** start. Its
`time` is the requested start and its `source_time` is the callback observation
time. That observation can be a libfst block-frame snapshot; it is not
necessarily the time of a signal transition. `value` and `source_time` are null
when no prior callback was available, including at trace start or an exact block
boundary. A recording gap after the prior observation also invalidates it.

All callbacks at start are then emitted as ordinary `event` records. Apply them
in `sequence` order to obtain the observed state at start. This avoids inventing
a prior state or collapsing same-timestamp callbacks. Equal-timestamp ordering
is libfst callback ordering, **not an HDL delta-cycle ordering guarantee**.
Events may include repeated values or frame snapshots; callback counts are not
automatically value-transition counts. If output is truncated, even the complete
set of callbacks at start may be absent.

The C reader's time-range option skips whole blocks; callbacks within an
included block can be earlier than start or later than end. `queryfst` retains
prior values for initial state and applies exact bounds itself. Fixed-size
signals can recover their starting value from block snapshots. Variable-length
signals have no such snapshots, so selecting any of them makes decoding start
at trace start. A prior recording gap also requires scanning from trace start:
a later block snapshot can otherwise hide the fact that a value predates the
gap. The header's `scan_start` exposes these potentially expensive fallbacks.

## Scalar summaries

`--summary` accepts only scalar state signals (one-bit logic, excluding VCD
events). It emits one `scalar_summary` per physical handle, with:

- `residency_ticks`: elapsed ticks spent in `0`, `1`, `x`, `z`, `other`, or
  `unavailable` state; the sum equals `end - start`.
- `value_transitions`: unequal consecutive observed values with a known prior
  value. Changes at both inclusive boundaries count; an initial observation from
  an unavailable state does not count as a transition.
- `callbacks`: inclusive in-range callback count, including repeated values.

Residency measures continuous elapsed time over `[start, end)`, so an event at
end can change the transition count but adds no duration. It is **not a cycle
count** or edge-sampled assertion count. Multiple same-timestamp transitions
add no elapsed time. Intervals intersecting disabled dumping are rejected for
summary mode because their state residency cannot be inferred. An observation
before a prior dumping gap cannot establish state after that gap. This includes
off/on pairs at the same timestamp, which can hide changes without consuming
any elapsed ticks. Summary mode also rejects an off entry at either inclusive
endpoint; the ordering of blackout metadata against callbacks is not known.

## Output and resource limits

`--limit` defaults to 10000 and bounds only `event` records. Zero emits initial
state and final counts without events. Metadata, initial states, dump activity
and summaries are not included in this limit. A large selected hierarchy or an
individual wide/variable value can therefore still produce substantial output.
Summary mode emits no events and ignores the event limit.

Truncation bounds output, not decoding work: libfst has no cancellation callback.
The traversal continues through the requested blocks to compute complete
callback counts. The header explicitly reports `early_termination: false`.
Application memory holds selected hierarchy names and one prior value per
selected signal, not the trace's event history. libfst additionally allocates
hierarchy and per-block decoding buffers; a narrow query is not a hard bound on
process RSS or CPU time.

Successful execution ends with a `summary` containing `complete: true`. An
argument, input, decoding, or output error exits nonzero and prints a diagnostic
to stderr. A decoding/output failure can leave a partial JSONL stream; consumers
must check exit status and the final completion record before trusting counts.
Limit truncation is a successful query and is explicitly marked `truncated`.

Tests run with `cargo test -p queryfst` and generate their small FST fixtures in
Cargo's ignored build directory.
