# fstapi

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

## License

Copyright (C) 2023 MaxXing. Licensed under either of [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
