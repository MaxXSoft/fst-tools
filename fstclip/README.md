# fstclip

[<img alt="github" src="https://img.shields.io/badge/github-MaxXSoft/fst--tools-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/MaxXSoft/fst-tools)
[<img alt="crates.io" src="https://img.shields.io/crates/v/fstclip.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/fstclip)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/MaxXSoft/fst-tools/build-test.yml?branch=master&style=for-the-badge" height="20">](https://github.com/MaxXSoft/fst-tools/actions?query=branch%3Amaster)

Create an FST waveform containing a selected time window and/or subset of signals.
The result is written to an output FST file. Successful clipping is silent; there
is no text summary or JSON output mode.

```sh
fstclip trace.fst window.fst --start 1000 --end 2000
fstclip trace.fst signals.fst --signals '(^|\.)(pc|valid)( \[[^]]+\])?$'
fstclip trace.fst selected-window.fst -s 1000 -e 2000 -S '^top\.core\.valid$'
fstclip trace.fst repacked.fst --pack-type z --repack
```

Without time or signal filters, the entire waveform is rewritten using the
selected compression settings. This is a waveform rewrite, not a byte-for-byte
file copy.

## Usage and options

```text
fstclip [OPTIONS] <INPUT> <OUTPUT>
```

Both paths are required. Input and output must refer to different files;
in-place clipping, including through a hard link or symbolic link to the input,
is rejected. An existing output file is replaced without prompting.

| Option | Meaning |
| --- | --- |
| `-s`, `--start T` | Inclusive start in raw FST ticks; defaults to the input trace start. |
| `-e`, `--end T` | Inclusive end in raw FST ticks; defaults to the input trace end. |
| `-S`, `--signals REGEX` | Select signal declarations by a Rust regular expression against their full dotted paths. |
| `-t`, `--strip-attrs` | Omit hierarchy attribute records. |
| `-n`, `--no-comp-hier` | Disable hierarchy compression. |
| `-P`, `--pack-type TYPE` | Value-change compression: `4` for LZ4 (default), `f` for FastLZ, or `z` for zlib. |
| `-r`, `--repack` | Repack the entire output waveform through gzip when closing it. |
| `-p`, `--parallel` | Enable parallel output waveform writing. |
| `-h`, `--help` | Print help; `--help` includes descriptions of the pack types. |
| `-V`, `--version` | Print the version. |

Hierarchy compression is enabled by default. Gzip repacking and parallel writing
are disabled by default. Repacking is independent of the value-change pack type;
the output remains an FST waveform.

## Time windows

`--start` and `--end` are unsigned decimal timestamps in **raw input FST ticks**.
They do not accept unit suffixes or include the input's `timezero` offset. One
tick represents `10 ** timescale_exponent` seconds; inspect the input with
`fstprobe trace.fst --metadata` to find its timescale and trace bounds.

Both endpoints must lie inside the input trace, and start must not exceed end.
The interval includes both endpoints. Equal endpoints are allowed, producing a
zero-duration clip.

Output timestamps are rebased to zero:

```text
output timestamp = input timestamp - start
output trace bounds = [0, end - start]
output timezero = input timezero + start
```

The timescale is preserved. For example, clipping `[1000, 2000]` from a trace
whose `timezero` is `-7` produces bounds `[0, 1000]` and `timezero = 993`.
An input callback at tick `1250` appears at output tick `250`. If the adjusted
`timezero` cannot fit in a signed 64-bit integer, clipping fails before creating
or replacing the output.

Earlier values provide the initial state at output tick zero when available.
A signal need not change within the selected window to be retained. Values at
the start boundary take precedence over earlier state, and multiple callbacks
at that boundary are retained. The output extends to `end - start` even if no
signal changes near the end.

Recording-enable changes are clipped and rebased too. If recording was already
disabled at the start, the output records that state at tick zero. A recording
gap does not establish that signal values remained constant during the gap.

## Signal selection

`-S` / `--signals` matches each declaration's **full dotted hierarchical path**,
consistent with the other tools. For example, `^top\.core\.valid$` selects only
`top.core.valid`, while `(^|\.)valid$` selects signals named `valid` in any scope,
including at the root. Scope names and the local declaration name are joined
with literal dots.

Matching is case sensitive and unanchored unless the expression contains
anchors. Escape literal dots as `\.` and quote expressions in the shell to
preserve regex metacharacters. Declaration range suffixes remain part of the
path: `(^|\.)pc$` does not match `top.core.pc [31:0]`, while
`(^|\.)pc( \[[^]]+\])?$` matches both that spelling and `top.core.pc`.

This replaces the previous local-name matching behavior. An anchored filter
such as `^valid$` now matches only a root-level declaration named `valid`.
Use `(^|\.)valid$` to retain matches in any scope, or specify a full path to
select one particular signal.

Without `--signals`, all declarations are retained. With a filter, matching
declarations retain their enclosing hierarchy, and scopes without selected
declarations are omitted. Each alias is matched by its own full path. If only an
alias matches, it becomes an ordinary declaration in the output with the same
signal values; multiple selected declarations sharing an input handle continue
to share a handle in the output. Output handle numbers can differ from input
handle numbers.

If the input has signals and none match the filter, the command fails with
`No matching signals!` rather than producing a successful empty selection.

## Preserved data

The output keeps the input's date, version, file type, and timescale metadata,
with `timezero` adjusted as described above. Selected declarations keep their
names, types, directions, and widths. Value copying supports logic values, real
values, EVCD port values and drive strengths, and variable-length data such as
strings, including embedded NUL bytes.

Hierarchy attribute records are copied by default, including binary source
attributes. `--strip-attrs` removes these records; it does not remove signal
declarations or values, or the file metadata listed above.

## Output and errors

Normal successful clipping writes no stdout or stderr messages. Diagnostics are
plain text on stderr; help and version requests produce ordinary CLI text.
There is no `--json` or `--format` option.

| Exit code | Meaning |
| --- | --- |
| `0` | Success, including help or version requests. |
| `1` | Execution failure, such as an unreadable input, no matching signals, or a timezero overflow. |
| `2` | Invalid arguments, such as a malformed regex, invalid time range, or output referring to the input file. |

Scripts should check the exit status before using the output. The output is
written directly, so a failure after creation can leave a partial file or
replace a previous output; file existence alone does not indicate success.
To inspect the resulting metadata or signals in a structured format, use
`fstprobe`:

```sh
fstclip trace.fst window.fst --start 1000 --end 2000 &&
  fstprobe window.fst --all --format json
```

Run `cargo test -p fstclip` for clipping, full-path selection, initial-value,
boundary, alias, compression, attribute, recording-gap, and error regressions.
Generated test fixtures stay in Cargo's ignored build directory.
