# queryfst

Inspect FST value changes or run a finite SQL query over sampled signal state.
Text output is the default. `--format json` (or `--json`) enables a versioned
JSON Lines stream; `jsonl` is an alias for the same format.

```sh
queryfst trace.fst --signal top.core.retire_valid --start 120000 --end 130000
queryfst trace.fst --signals 'top\.core\.(stall|retire_valid)$' --summary --json
queryfst trace.fst --signal top.core.retire_pc --json --max-rows 100 --max-bytes 65536
```

Event selection uses repeated `--signal PATH` and/or `-S REGEX` (`--signals`).
Their union selects physical handles, so aliases never duplicate callbacks.
Missing exact paths and empty regex selections are errors. Anchoring a regex
with `^` and `$` requests a whole-path match.

## SQL sampling

SQL can refer directly to an exact FST hierarchy path inside double quotes:

```sh
queryfst trace.fst --period 2 --sql \
  'SELECT tick, raw("top.core.pc [31:0]") FROM samples
   WHERE "top.core.retire_valid" = 1' --json
```

The entire dotted path is one quoted identifier; `samples."top.core.pc [31:0]"`
is also supported. Embedded double quotes are doubled as in SQL identifiers.
Paths and binding identifiers are case-sensitive, with no suffix or fuzzy
matching. Missing or ambiguous paths are errors. Only referenced quoted paths
are added to the decode mask, and aliases sharing a handle share decoding.

For shorter names, use a JSON binding object or repeated `--bind NAME=PATH`.
An explicit binding takes precedence over an equally named quoted path.
`tick` and `sample_index` remain virtual columns even when quoted; use a binding
with another name to refer to a root signal with either reserved name.
`ORDER BY` output aliases keep their existing precedence over input columns.
The JSON header includes the SQL text and explicit bindings for reproducibility.

For example, save this object as `bindings.json`:

```json
{
  "pc": "top.core.retire_pc",
  "retired": "top.core.retire_valid",
  "reset": "top.reset",
  "mode": "top.core.mode",
  "events": "top.core.perf_events"
}
```

```sh
queryfst trace.fst --bindings bindings.json --period 2 --phase 0 \
  --sql 'SELECT mode, COUNT(*) AS cycles, SUM(retired) AS retired_count
         FROM samples WHERE reset = 0 GROUP BY mode' --json

queryfst trace.fst --bindings bindings.json --period 2 --phase 0 \
  --sql 'SELECT pc, COUNT(*) AS retired_count, MIN(tick) AS first_tick,
                MAX(tick) AS last_tick
         FROM samples WHERE reset = 0 AND retired = 1
         GROUP BY pc ORDER BY retired_count DESC LIMIT 20' --json
```

Use `--sql-file FILE` for larger queries. Every query reads the virtual `samples`
table. `tick` is the raw timestamp and `sample_index` is a zero-based ordinal in
the requested sampling window. These names cannot be rebound.

Sampling is explicit: `--period P --phase Q` selects ticks satisfying
`t % P == Q`, with `P > 0` and `0 <= Q < P`. Phase is anchored to absolute trace
time, not to `--start`. All selected callbacks at a timestamp are applied before
a sample at that timestamp. Constant signals are carried forward, and samples
are generated even when no selected signal changes. This differs from counting
callbacks or PC transitions. Periodic sampling assumes the caller knows the
clock cadence; it does not infer edges, gated clocks, HDL delta cycles, or the
meaning of retirement from signal names.

For the Fuxi example, the measured contract was period 2, phase 0: settled
low-phase snapshots before the next odd rising edge. That choice is specific to
that simulation. Other testbenches may require a different period/phase.

### Supported SQL

One `SELECT ... FROM samples` statement supports:

- `WHERE`, `GROUP BY`, `ORDER BY` and integer `LIMIT`.
- `COUNT(*)`, `COUNT(expr)`, `SUM`, `MIN`, and `MAX`.
- Integer arithmetic, comparisons, boolean operations, bitwise operators,
  `CASE`, `BETWEEN`, `IN`, and null tests.
- Explicit projected expressions and `AS` names. `SELECT *` is intentionally
  rejected so the result schema and selected data are reviewable.
- `raw(value)`, `known(value)`/`is_known(value)`, `bit(value, index)`, `hex`,
  `abs`, and `coalesce`.

Joins, subqueries, CTEs, DDL/DML, window `OVER` clauses, `HAVING`, `DISTINCT`,
floating point expressions, and unsupported modifiers are rejected. Parsing
uses `sqlparser`; execution is a small streaming Rust engine, not a database.
SQL integers use checked signed 128-bit arithmetic. Known logic vectors are
interpreted as unsigned values; signed RTL interpretation is not inferred.
Integer division truncates toward zero. Arithmetic errors are explicit errors.
Only fixed-width logic bindings are accepted in sampling mode. Event-query mode
also supports real, EVCD and variable-length values.

Four-state values remain available as bit strings. Unknown numeric operands
propagate NULL, so WHERE accepts only definite true. `known(value)` distinguishes
unknown/unobserved values; `raw(signal)` retains width and leading zeroes.
Numeric operations on known values exceeding the signed 128-bit range fail
rather than truncate. Numeric ordering/group keys and `changed` use the same
comparison range; use `changed(raw(wide_signal))` or group/order by `raw(wide_signal)`
for exact arbitrary-width bit-string comparisons. All explicit bindings are
decoded, even when the SQL references only some of them; small binding maps
reduce scanning work. Direct quoted paths are discovered from SQL expressions. Temporal expressions begin at the requested sample window;
the prior waveform state is reconstructed, but earlier handshakes/history are
not replayed into those expressions.

### Temporal predicates and context

These functions advance once per sampled row **before WHERE**, including rows
that the predicate rejects. Repeated uses of the same expression share its state.

| Function | Meaning |
| --- | --- |
| `lag(value)` | Value on the previous sample; initially NULL |
| `changed(value)` | Value differs from the previous sample, including first known observation |
| `hold(value, enable)` | Most recent value accepted when enable was true, including this sample |
| `run_length(predicate)` | Consecutive true samples; false resets to zero; unknown resets and returns NULL |
| `runs(predicate)` | True only on the first sample of a true run |
| `timeouts(request, response, cycles [, key])` | Count of pending accepted requests whose inclusive response deadline expires on this sample |

For example, find the tenth cycle of each `valid && !ready` run:

```sh
queryfst trace.fst --bind valid=top.valid --bind ready=top.ready --period 2 \
  --sql 'SELECT tick, valid, ready FROM samples
         WHERE run_length(valid = 1 AND ready = 0) = 10' --before 5 --after 3 --json
```

`--before` and `--after` include neighboring samples for projection queries,
append a `__match` boolean, and merge overlapping windows without duplicate rows.
They are sample counts, not raw tick distances. `--matches first|last|all` selects
the trigger before expanding its context (default `all`). Context is clipped to the
requested time window, and the summary reports unavailable leading/trailing
context. Aggregation and nonchronological sorting cannot be combined with context.

For first/last results without context, use ascending `ORDER BY tick LIMIT 1`,
descending `ORDER BY tick LIMIT 1`, or `--matches first|last|all`. For context,
use `--matches first --before 20 --after 10`, for example. SQL LIMIT is rejected
with context/match selection so it cannot accidentally consume the preceding
context rows and omit the trigger. First mode stops after its following window;
last mode retains a bounded candidate window until the scan completes. A partial
last-match result is provisional. A condition that remains true matches every
sampled row; use `runs(condition)` for one result per episode.

For a ready/valid bus, pass accepted handshakes to `timeouts`, not bare valid
levels. The optional key pairs responses with the oldest pending request of the
same key. A response on the deadline sample succeeds; a simultaneous request and
response can complete immediately. At EOF, pending requests are unresolved, not
reported as timeouts. Unknown handshake/key values invalidate pending evidence;
the summary exposes that uncertainty. Pending request storage shares the
`--max-buffer-rows` cardinality budget. Timed-out requests retain bounded FIFO
placeholders until a response arrives, so a late response does not satisfy a
newer request. The optional key applies to both channels on that sample; buses
with different simultaneous request/response IDs need an appropriate extraction
or separate per-ID queries.

A UART with separate address/data handshakes can be queried with
`hold(awaddr, awvalid = 1 AND awready = 1)` in the data-channel predicate. Callers
must choose a transaction model appropriate to their bus; this primitive does
not infer AXI ordering or transaction IDs.

## Boundaries and recording gaps

`--start` and `--end` are inclusive raw FST ticks and default to trace bounds.
Reversed or out-of-trace windows are errors. Point queries are supported.
Timescale and timezero are metadata; timezero is not added to callback timestamps.

Raw `initial` records contain the latest callback **strictly before** start.
Their `source_time` can be a libfst block-frame snapshot, rather than an actual
transition. Unknown initial state is null. All callbacks at start follow as
ordinary events in libfst callback order. This order does not recover HDL delta
cycles. Repeated values and block snapshots can occur, so callbacks are not
necessarily transitions.

The backend skips whole blocks; the tools enforce exact bounds on callbacks.
Variable-length signals have no frame snapshots, so raw queries selecting them
scan from the trace start. A prior dump gap also requires reconstruction from
trace start, invalidating observations that predate disabled recording. SQL
sampling and scalar summaries reject recording interruptions inside the requested
window, including zero-duration off/on pairs. Unknown history is never silently
converted to zero.

`--summary` in event-query mode accepts one-bit state signals, excluding VCD events.
`residency_ticks` integrates `[start,end)` and sums to `end-start`; transitions
include both endpoints. This is elapsed residency, not sampled cycle counting.
A work-budget interruption omits these summaries rather than claiming a final
residency over an unobserved interval.

## Resource budgets and completion

| Option | Controlled resource |
| --- | --- |
| `--max-rows N` / `--limit N` | Output data rows; default 10000, excluding metadata/footer |
| `--max-bytes N` | Total serialized stdout, including metadata/footer; minimum 4096 |
| `--max-callbacks N` | Delivered value callbacks, including initial-state reconstruction |
| `--max-samples N` | Sampled rows evaluated, including rows rejected by WHERE |
| `--max-groups N` | Distinct aggregation keys; default 100000 |
| `--max-buffer-rows N` | Buffered sort/context/pending-request row cardinality; default 100000 |
| `--max-duration-ms N` | Cooperative scan/sample duration budget |

Bytes are checked on whole records, reserving 2048 bytes for a final summary.
No line is split. A large metadata record can itself consume the available
budget; inspect `output_truncated` before assuming every signal/column was emitted.
Token budgets and continuation are not implemented.

SQL LIMIT is part of the query, whereas `--max-rows` is an output budget. Top-PC
rankings and final aggregates need a complete window even if only one row is
returned. A raw row/byte output cap continues scanning for exact callback counts;
SQL projection output caps can stop traversal. Work limits stop the underlying C
reader through a build-local cancellation extension. Loading/decompressing a
block is not preempted, so the deadline is cooperative, not a hard wall-time
limit. Opening/hierarchy discovery, SQL planning, final sorting and result
serialization are outside this duration budget. These limits do not cap process RSS, hierarchy sizes, individual value
widths, SQL expression sizes, or libfst's block buffers. Group/row limits bound
cardinality, not allocated bytes.

A final `summary` separates `complete`, `output_truncated`, `aggregate_final`,
and `unprocessed_input`. A partial aggregate describes only observed samples,
not the entire requested range. An absence of matches in a partial result does
not establish an absence in the input. `processed_through` identifies observed
sample coverage; callbacks at a budget-interrupted timestamp are not sampled
until the entire timestamp is known. No continuation token is supplied.

## JSON Lines schema version 2

Every stdout line is an object with a `type` field:

- Raw mode: `header`, `signal`, `dump_activity`, `initial`, `event` or
  `scalar_summary`, then `summary`.
- Sampling mode: `header`, `columns`, zero or more `row` records, then `summary`.
  A row's `values` array follows the ordered `columns` names. Empty results still
  provide columns. Integers, times and large counters are decimal strings;
  booleans remain booleans, missing values are null, and unknown/wide values
  preserve a `{"bits":"..."}` representation.

Raw values use `bits`, `bytes_hex`, `real_f64_le_hex`, or `evcd` encodings.
Native real callbacks preserve the exact eight IEEE-754 bytes in little-endian
hex; strings preserve arbitrary bytes as hex. Handles/widths/schema identifiers
remain JSON numbers. Version 2 adds explicit budgets and SQL result records;
consumers of the previous JSON-only default must now pass `--format json`.

Argument/query errors produce a structured `error` on stderr in JSON mode and
exit nonzero. A decoding/output failure may leave an unfinished stdout stream.
Consumers must check both exit status and the final summary. Budget-limited
queries exit successfully with explicit partial/truncated status.

Run `cargo test -p queryfst` for generated waveform regressions. Large traces and
experimental result tables remain outside the repository's tracked fixtures.
