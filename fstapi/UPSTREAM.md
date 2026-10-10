# Maintaining libfst

The FST implementation is based on the unmodified
[gtkwave/libfst](https://github.com/gtkwave/libfst) Git submodule at
`third_party/libfst`. The submodule gitlink is the authoritative revision; builds
do not fetch or follow the upstream branch. Some reviewed compatibility corrections
are applied to a build-local source copy, as described below.

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

The pinned engine writes `signal_typs[maxhandle]` when it encounters a real
alias in `fstReaderProcessHier`. That index denotes the next unique handle,
and is out of bounds at a capacity boundary (65,536 handles and its doublings).
The alias already refers to an existing facility, so the assignment is redundant.
This function is used both by VCD export and by opening zero-duration files;
a VCD-only preflight cannot protect all callers.

`patches/real-alias.patch` records the single-line removal. Its context includes
the alias branch so the similar assignment for non-alias variables cannot match
instead. Retain the open/read/export boundary regressions when updating this fix.

The pinned engine also uses local `const int` values as array bounds in
`fstReaderVarint32`, `fstReaderVarint32WithSkip`, and `fstReaderVarint64`.
These are variable-length arrays in C, which MSVC does not support.
`patches/msvc-varint.patch` changes the three declarations to enum constants,
preserving the 5-, 5-, and 16-byte buffers, read limits, and TALOS-2023-1783 checks.

The pinned engine restricts several Windows file-handling workarounds to MinGW.
MSVC consequently tries to unlink open hierarchy and unpacked files, leaving
them behind, and uses `fflush` on input streams before handing their duplicated
descriptors to zlib. UCRT does not synchronize input streams that way, so opening
a repacked FST or reading a gzip hierarchy can fail.
`patches/windows-stdio.patch` includes MSVC in the unbuffered reader and deferred
file-deletion paths. It retains named hierarchy scratch files until close and
explicitly synchronizes both gzip input descriptors with `_lseeki64`, including
hierarchies beyond the 32-bit offset range. Uncompressed hierarchy sidecars remain
part of the output.

Closing a file in the parent process is insufficient if another thread has
started a child that inherited its handle. Windows file opens therefore use
the CRT's `N` mode to disable inheritance at creation time. The Windows temporary
file path is also used by MSVC with `D`/`N` modes, preserving deletion on close
without inheritable `tmpfile` handles. Gzip descriptors use `DuplicateHandle`
with inheritance disabled before transferring ownership to `_open_osfhandle`;
plain `_dup` can create an inheritable handle again. Setting handle flags after
opening or duplicating would still race with concurrent process creation.
Temporary files also retain the empty file reserved by `GetTempFileName` until
`fopen` opens it. Unlinking that reservation first allows another concurrent
writer to reuse the same name, causing sharing failures or scratch-file collisions.
The VCD export adapter uses `N` for its output stream as well. The mmap paths
remain unchanged.
See Microsoft's [input `fflush` semantics](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/fflush)
and [64-bit descriptor seeking](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/lseek-lseeki64),
[`fopen` mode flags](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/fopen-wfopen),
and [`DuplicateHandle` inheritance](https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-duplicatehandle).

These patches are standard unified diffs with paths relative to
`third_party/libfst`. The build script normalizes checkout line endings and uses
the Rust `diffy` crate to apply all patches, in the listed order, to
`OUT_DIR/fstapi.c` on every target; no external patch command is required.
All context lines must match, although line offsets may change. An invalid patch,
an unexpected hunk count, or a context mismatch fails the build and identifies
the patch. All patches are included in published crates. The submodule and public
C ABI remain unchanged. Remove each correction when its fix is available in the
reviewed upstream pin.

`patches/reader-cancel.patch` adds the project-owned
`fstToolsReaderIterBlocksControlled` entry point. The original reader entry point
delegates with a null cancellation flag, preserving its ABI. Rust callbacks can
set a same-thread flag; the reader checks it during frame delivery and value
iteration, then follows normal buffer cleanup. This powers
`Reader::for_each_block_controlled`, which returns whether traversal exhausted
its input. Callback panics and validation errors use the same cleanup path.
Cancellation does not preempt block loading/decompression, bound backend memory,
or provide a resumable cursor. The fixed, variable-length, frame-snapshot, panic,
and repeated-reader-use regressions must accompany changes to this patch.

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
  `LimitExceeded` without emitting them. Its kind distinguishes an oversized
  record payload from a full section; the error includes the requested size
  and the applicable limit in bytes.
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

Actual engine fixes should be proposed upstream. The documented corrections
are the only build-time exceptions; do not silently edit the submodule,
add undisclosed corrections, or fetch sources during a Cargo build.

## Tests

The crate tests generate small waveforms beneath Cargo's test artifact directory
and remove them afterwards. They cover compression/hierarchy/repack combinations,
including writer and reader scratch-file cleanup while retaining required sidecars,
and cleanup while an unrelated child process is still alive,
real and string values, aliases, EVCD ports, masks, time ranges, invalid input,
initial values, VCD output, retained hierarchy data, and callback panics.
Each CLI has integration tests using generated waveforms. Large simulator traces
and external simulation sources are validation inputs, not repository fixtures.
The C adapter formatting check runs only on the Linux CI job. Upstream C sources
are excluded from that check and retain their original formatting.
Linux CI also builds the C sources with GCC and `-Werror=vla` to catch MSVC
portability regressions. The Windows CI job continues to build, test, and verify
the packaged crate with the default MSVC toolchain.

The Rust wrapper is MIT OR Apache-2.0; libfst and its bundled compression code
retain their own notices in `third_party/libfst/LICENSE` and the source headers.
