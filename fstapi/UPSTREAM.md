# Maintaining libfst

The FST implementation is the unmodified
[gtkwave/libfst](https://github.com/gtkwave/libfst) Git submodule at
`third_party/libfst`. The submodule gitlink is the authoritative revision; builds
do not fetch or follow the upstream branch.

Initialize a source checkout before building:

```sh
git submodule update --init --recursive
cargo test --workspace
```

The published `fstapi` crate includes the required upstream C sources, headers,
and license. Crate users do not need Git, Meson, or a separate libfst install.
The existing C compiler, zlib, pthreads, and libclang build requirements remain.
Windows MSVC additionally uses the vcpkg packages described in the
[root README](https://github.com/MaxXSoft/fst-tools/blob/master/README.md).

## Local adaptation

`build.rs` compiles upstream `fastlz.c`, `fstapi.c`, and `lz4.c` with `cc`.
It supplies the platform configuration, including both `HAVE_LIBPTHREAD` and
`FST_WRITER_PARALLEL`, and MSVC's 64-bit file-position mappings. Bindings are
generated from the same upstream header used to compile the library.

Keep upstream files unchanged. Local responsibilities are:

* `src/writer.rs`: validate handles, normalized value widths, aliases, EVCD
  encoded widths, variable-length values, and nondecreasing timestamps before
  calling the upstream `void` emitters. The first value emission initializes
  time zero if no explicit timestamp was supplied. Each successful emission is
  a change at that timestamp, including successive changes at time zero. This
  preserves static initial values and initial strings that the old wrapper
  could lose. A null `date_raw` leaves the date unchanged.
  Value changes also consume a conservative one-GiB budget per section,
  including record overhead, to keep libfst's 32-bit buffer calculations and
  signed compression lengths in range. The budget resets when a time change
  consumes an automatic or requested flush; requesting a flush alone does not
  reset it. At 512 MiB of budget usage, time changes proactively request a flush
  so long traces can continue in smaller sections. This is a per-section limit,
  not a limit on the total FST file size. Oversized changes return
  `InvalidOperation` without emitting them.
  Raw C-string attributes preserve binary source indices, and dump activity
  can be emitted independently of signal values.
* `src/reader.rs`: own hierarchy records instead of retaining references into
  libfst's reused storage; recover callback lengths using per-handle metadata
  and native-double mode; catch callback panics until the C traversal returns.
  Expose raw attribute bytes and all dump activity transitions for lossless
  metadata copying, independently of signal masks and time limits.
* `csrc/fst_tools.c`: manage C `FILE *` for VCD export. Hierarchy processing
  clears libfst's process mask, so export explicitly enables every signal before
  writing values. The shim checks stream and close/flush errors.
  It also preflights the hierarchy for the pinned engine's real-alias overflow:
  a real alias at exactly 65,536 unique handles, or any doubled table capacity,
  returns `InvalidOperation` before the output file is opened. This deliberately
  limits VCD export for that shape of otherwise valid FST file; ordinary reading
  and clipping remain available. Remove the preflight after incorporating an
  upstream fix, retaining its boundary regression tests.

The public Rust `Result` DOES NOT promise recovery from every C failure.
libfst still has fatal internal error paths, and its `void` close operation
CANNOT report all write failures through Rust's `Drop`.

## Updating the pin

1. Fetch upstream and review the changes since the current gitlink, including
   API/header, compression, platform, and license changes.
2. Check out the reviewed commit in `third_party/libfst` with a detached HEAD.
3. Run the workspace checks and package verification from the repository root:

   ```sh
   cargo fmt --all --check
   clang-format --dry-run --Werror fstapi/csrc/*.c fstapi/csrc/*.h
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace
   cargo package -p fstapi --list
   cargo package -p fstapi
   ```

   Before committing an update, `cargo package --allow-dirty -p fstapi` can
   verify the pending sources. Inspect the file list for `third_party/libfst/src/`,
   `third_party/libfst/LICENSE`, and the local shim. Packaging verifies a fresh build
   from the extracted crate, without relying on the submodule checkout.
4. Check Linux, macOS, and Windows MSVC CI. For changes to the reader or writer,
   also compare values and metadata from an independently generated waveform.
5. Commit the gitlink update with any necessary wrapper changes and record the
   upstream revision and validation in the commit description.

Actual engine fixes should be proposed upstream; do not silently edit the
submodule or fetch/patch sources during a Cargo build.

## Tests

The crate tests generate small waveforms beneath Cargo's test artifact directory
and remove them afterwards. They cover compression/hierarchy/repack combinations,
real and string values, aliases, EVCD ports, masks, time ranges, invalid input,
initial values, VCD output, retained hierarchy data, and callback panics.
Each CLI has integration tests using generated waveforms. Large simulator traces
and external simulation sources are validation inputs, not repository fixtures.
The C adapter formatting check runs only on the Linux CI job. Upstream C sources
are excluded from that check and retain their original formatting.

The Rust wrapper is MIT OR Apache-2.0; libfst and its bundled compression code
retain their own notices in `third_party/libfst/LICENSE` and the source headers.
