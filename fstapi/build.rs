use std::{env, path::PathBuf};

fn main() {
  #[cfg(windows)]
  let (zlib_include_dir, mman_include_dir) = {
    // Find zlib via vcpkg.
    let zlib = vcpkg::Config::new()
      .emit_includes(true)
      .find_package("zlib")
      .unwrap();
    let zlib_include_dir = zlib.include_paths[0].clone();

    vcpkg::Config::new().find_package("pthreads").unwrap();

    let mman = vcpkg::Config::new().find_package("mman").unwrap();

    // vcpkg installs mman under `mman/sys/mman.h` path structure.
    // We need to include the parent `mman` directory so that
    // fstapi.c can find `sys/mman.h` without modification.
    let mman_base_path = mman.include_paths[0].clone();
    let mman_include_dir = mman_base_path.join(PathBuf::from("mman"));

    (zlib_include_dir, mman_include_dir)
  };

  // Compile C sources to library.
  let mut cc_build = cc::Build::new();
  cc_build
    .files(["csrc/fastlz.c", "csrc/fstapi.c", "csrc/lz4.c"])
    .define("FST_WRITER_PARALLEL", None)
    .include("csrc")
    .flag_if_supported("-Wno-unused-but-set-variable");

  #[cfg(windows)]
  cc_build
    .include(&zlib_include_dir)
    .include(&mman_include_dir);

  cc_build.compile("fst");

  // Rebuild if C source changes.
  println!("cargo:rerun-if-changed=csrc");

  // Link with zlib.
  #[cfg(not(windows))]
  println!("cargo:rustc-link-lib=z");

  // Generate bindings.
  let bindgen_builder = bindgen::Builder::default()
    .header("csrc/fstapi.h")
    .allowlist_type(r#"(fst|FST_)\w+"#)
    .allowlist_function(r#"(fst|FST_)\w+"#)
    .allowlist_var(r#"(fst|FST_)\w+"#)
    .clang_arg("-Icsrc");

  #[cfg(windows)]
  let bindgen_builder = bindgen_builder.clang_arg(format!("-I{}", zlib_include_dir.display()));

  let bindings = bindgen_builder
    .generate()
    .expect("failed to generate bindings");

  // Write the bindings to file.
  let out_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
  bindings
    .write_to_file(out_path)
    .expect("failed to write bindings");
}
