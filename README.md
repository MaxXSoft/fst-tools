# fst-tools

[<img alt="github" src="https://img.shields.io/badge/github-MaxXSoft/fst--tools-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/MaxXSoft/fst-tools)
[<img alt="crates.io" src="https://img.shields.io/crates/v/fstapi.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/fstapi)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-fstapi-66c2a5?style=for-the-badge&labelColor=555555&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/fstapi)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/MaxXSoft/fst-tools/build-test.yml?branch=master&style=for-the-badge" height="20">](https://github.com/MaxXSoft/fst-tools/actions?query=branch%3Amaster)

Tools for manipulating Fast Signal Trace (FST) format waveforms.

FST is an open source file format for storing digital waveforms from HDL simulations. It was created by the author of [GTKWave](https://github.com/gtkwave/gtkwave) in 2014, as an alternate to the [VCD](https://en.wikipedia.org/wiki/Value_change_dump) (Value Change Dump) format.

For more details, please see:

* The [source code](https://github.com/gtkwave/gtkwave/tree/e1c01753bc5db9f7b42e41b9bde651a375ec5eba/gtkwave4/src/helpers/fst) of GTKWave.
* The [documentation](https://gtkwave.sourceforge.net/gtkwave.pdf) of GTKWave.
* An [unofficial specification](https://blog.timhutt.co.uk/fst_spec/) for FST format.

## Available Tools

* [`readfst`](readfst): tool for displaying information about the contents of FST waveform, like `readelf`.
* [`findfst`](findfst): tool for finding values of signals from FST waveform, like `fstminer` tool that comes with GTKWave but more powerful.
* [`clipfst`](clipfst): tool for clipping from FST waveform.

## Building on Windows

Windows requires additional setup due to dependencies that are not readily available:

### Prerequisites

- [Rust](https://rustup.rs/) (latest stable version)
- [vcpkg](https://github.com/Microsoft/vcpkg) package manager
- Visual Studio Build Tools or Visual Studio Community

### Build Instructions

1. **Install vcpkg:**

   It comes preinstalled with Visual Studio, but if you don't have it yet, you can install it by running the following commands in PowerShell:

   ```powershell
   git clone https://github.com/Microsoft/vcpkg.git
   cd vcpkg
   .\bootstrap-vcpkg.bat
   ```

   (tested with vcpkg version 2025-02-11-bec4296bf5289dc9ce83b4f5095943e44162f9c2)

2. **Install required packages:**

   ```powershell
   vcpkg install zlib:x64-windows-static-md
   vcpkg install pthreads:x64-windows-static-md
   vcpkg install mman:x64-windows-static-md
   ```

   (adjust vcpkg path as necessary, depending on whether it is on your PATH or not)

3. **Build**

   ```powershell
   cargo build
   ```

**Note:** The build requires the `x64-windows-static-md` triplet for vcpkg packages to ensure compatibility with Rust's MSVC toolchain.

### Troubleshooting

```
  thread 'main' panicked at fstapi\build.rs:19:8:
  called `Result::unwrap()` on an `Err` value: LibNotFound("package zlib is not installed for vcpkg triplet x64-windows-static-md")
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

This error indicates that the `zlib` package is not found for the specified vcpkg triplet. Ensure that you have installed `zlib` using the correct triplet as shown in step 2 above. Similarly for pthreads or mman packages.

Note that we're using pthreads (with an s) instead of pthread (without s) because the latter is deprecated in vcpkg.

## Rust Wrapper for FST C API

This repository contains a Rust wrapper for the FST C API provided by GTKWave. See the [`fstapi`](fstapi) directory.

All of the tools in the repo are written in Rust using this wrapper.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## License

Copyright (C) 2023-2025 MaxXing. Licensed under either of [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
