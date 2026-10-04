# readfst

Inspect FST metadata and hierarchy without decoding signal value changes.
The default output is the existing CLI table format. Select the sections you
need with `--metadata`, `--vars`, `--scopes`, or `--attrs`; `--all` selects all four.

```sh
readfst design.fst --metadata --format json
readfst design.fst --vars --format json --signals '(^|\.)(pc|inst|valid)$'
readfst design.fst --vars --names-only --signals 'cache.*miss'
readfst design.fst --all --no-aliases --format json
```

`--signals REGEX` matches full dotted variable names using Rust regular expression
syntax. Matching is case sensitive and unanchored unless the expression contains
anchors. It requires `--vars` or `--all`, and filters only the variables section.
FST names retain any declaration range suffix, such as `pc [31:0]`; a suffix
match such as `pc( \[[^]]+\])?$` includes both that spelling and plain `pc`.
`--no-aliases` removes alias declarations after matching each declaration's own
name. An alias can match even when its canonical declaration does not.
`--names-only` is supported only with table output.

## JSON schema version 1

JSON mode writes one compact JSON object and a trailing newline to stdout.
Diagnostics go to stderr; argument errors exit with code 2, and read/write errors
exit with code 1. Successful output exits with code 0. Consumers must check the
exit status before accepting a document, particularly after an output I/O error.

The top-level fields are `schema: "readfst"`, `schema_version: 1`, `file` (the input
path as supplied), and the selected `metadata`, `variables`, `scopes`, and
`attributes` sections. Unselected sections are omitted. Selected empty hierarchy
sections are empty arrays. Array ordering follows FST hierarchy order and remains
stable between repeated invocations against the same file; object member order is
not a consumer contract. Future compatible releases may add fields.

### Metadata

`metadata` contains `date`, `version`, `file_type`, `timescale`, `timezero`,
`start_time`, `end_time`, `num_scopes`, `num_vars`, `num_aliases`,
`timescale_exponent`, and `file_type_code`.

- `timescale_exponent` is a JSON integer: one tick is
  `10 ** timescale_exponent` seconds. `timescale` retains the readable unit label.
- `start_time` and `end_time` are raw FST ticks, stored as decimal strings.
  `timezero` is a signed decimal string containing the FST time-zero offset.
  The tool does not apply the offset to reported timestamps.
- Counts are decimal strings too; `num_vars` counts declarations, including
  aliases. Metadata counts describe the complete file, unaffected by filters.
- `file_type_code` preserves the underlying FST enum value.

### Variables

Each variable has `handle`, `type`, `direction`, `name`, `width`, `alias_of`,
`hierarchy_index`, `path`, `canonical_name`, `is_alias`, `type_code`, and
`direction_code`.

- `handle` is a positive unsigned 32-bit JSON integer identifying the physical
  signal. Aliases share their canonical declaration's handle.
- `name` is the full dotted name. `path` is an array of literal scope names
  followed by the literal variable name, preserving dots inside individual names.
- `is_alias` is a boolean. `alias_of` is the full canonical name for aliases and
  `null` for original declarations. `canonical_name` is present for both, including
  when the original declaration was filtered out.
- `width` is the unsigned 32-bit FST declaration length. For ordinary logic
  signals this is their bit width. Special FST types, such as real, string, and
  EVCD port values, retain their native FST length convention.
- `type` and `direction` are readable enum labels; the corresponding `_code`
  fields are their numeric FST enum values.
- `hierarchy_index` is a zero-based decimal string counting all hierarchy records,
  including scope exits and attribute records. It is unaffected by filtering.

Example variable (the exact hierarchy index depends on the input file):

```json
{"handle":1,"type":"VcdReg","direction":"Output","name":"top.pc","width":32,"hierarchy_index":"1","path":["top","pc"],"alias_of":null,"canonical_name":"top.pc","is_alias":false,"type_code":5,"direction_code":2}
```

### Scopes and attributes

A scope has its local `name`, `component`, readable `type`, numeric `type_code`,
`full_name`, literal component `path`, and decimal-string `hierarchy_index`.

Each attribute record contains `hierarchy_index`, `scope_path`, and `event`
(`"begin"` or `"end"`). Begin records also have `data`, containing `type`, `subtype`,
`name`, `arg`, and `arg_from_name`. Both numeric arguments are decimal strings.
Binary source-stem names have an empty `name`; their decoded source-file index is
in `arg_from_name`. End records omit `data`. The raw order and boundaries are
preserved; `scope_path` denotes the current hierarchy context and does not infer
which subsequent variable or scope an attribute describes.

## Cost and limits

Metadata-only output reads no hierarchy records. Other JSON requests walk the
hierarchy once, even when selecting several sections, and retain selected records
plus the canonical variable-name map in memory. No value-change records are
decoded. Memory use therefore scales with hierarchy size, rather than waveform
duration. JSON mode does not yet offer pagination or a streaming record format.
Names must be valid UTF-8 as required by the current fstapi text API.
