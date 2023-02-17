# fstapi

[<img alt="github" src="https://img.shields.io/badge/github-MaxXSoft/fst--tools-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/MaxXSoft/fst-tools)
[<img alt="crates.io" src="https://img.shields.io/crates/v/fstapi.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/fstapi)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-fstapi-66c2a5?style=for-the-badge&labelColor=555555&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/fstapi)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/MaxXSoft/fst-api/build-test.yml?branch=master&style=for-the-badge" height="20">](https://github.com/MaxXSoft/fst-api/actions?query=branch%3Amaster)

Rust wrapper of APIs for manipulating Fast Signal Trace (FST) format waveforms.

FST is an open source file format for storing digital waveforms from HDL simulations. It was created by the author of [GTKWave](https://github.com/gtkwave/gtkwave) in 2014, as an alternate to the [VCD](https://en.wikipedia.org/wiki/Value_change_dump) (Value Change Dump) format.

For more details, please see:

* The [source code](https://github.com/gtkwave/gtkwave/tree/e1c01753bc5db9f7b42e41b9bde651a375ec5eba/gtkwave4/src/helpers/fst) of GTKWave.
* The [documentation](https://gtkwave.sourceforge.net/gtkwave.pdf) of GTKWave.
* An [unofficial specification](https://blog.timhutt.co.uk/fst_spec/) for FST format.

## Usage

Add `fstapi` to your projects by running `cargo add`:

```
cargo add fstapi
```

## Examples

Create an FST waveform:

```rust
use fstapi::{Writer, var_type, var_dir};

// Create the waveform.
let mut writer = Writer::create("hello.fst", true)?
  .comment("FST waveform example")?
  .timescale_from_str("1ns")?;

// Create a variable.
let var = writer.create_var(var_type::VCD_REG, var_dir::OUTPUT, 8, "var", None)?;

// Emit value change data and time change data.
writer.emit_value_change(var, b"10001000")?;
writer.emit_time_change(10)?;
writer.emit_value_change(var, b"10011100")?;
writer.emit_time_change(42)?;
writer.emit_value_change(var, b"00111001")?;
writer.emit_time_change(100)?;
```

Print all variables of an FST waveform:

```rust
let mut reader = fstapi::Reader::open("hello.fst")?;
for var in reader.vars() {
  let (name, _) = var?;
  println!("{name}");
}
```

## More Examples

See the GitHub repository: [fst-tools](https://github.com/MaxXSoft/fst-tools), which contains 3 command line tools with this library for manipulating FST waveforms.

## License

Copyright (C) 2023 MaxXing. Licensed under either of [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
