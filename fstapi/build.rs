use std::{env, path::PathBuf};

fn main() {
  let upstream = PathBuf::from("vendor/libfst/src");
  assert!(
    upstream.join("fstapi.h").is_file(),
    "libfst sources are missing; run `git submodule update --init --recursive`"
  );

  // Build scripts run on the host, so use Cargo's target configuration.
  let target_family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
  let is_windows = target_family.split(',').any(|family| family == "windows");
  let is_unix = target_family.split(',').any(|family| family == "unix");

  let windows_includes = if is_windows {
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

    Some((zlib_include_dir, mman_include_dir))
  } else {
    None
  };

  // Compile C sources to library.
  let mut cc_build = cc::Build::new();
  cc_build
    .files(["fastlz.c", "fstapi.c", "lz4.c"].map(|file| upstream.join(file)))
    .file("csrc/fst_tools.c")
    .define("HAVE_LIBPTHREAD", None)
    .define("FST_WRITER_PARALLEL", None)
    .include(&upstream)
    .include("csrc")
    .flag_if_supported("-Wno-unused-but-set-variable");

  if is_unix {
    cc_build
      .define("HAVE_FSEEKO", None)
      .define("HAVE_REALPATH", None);
  }

  // The upstream Windows header supplies other CRT mappings, but these
  // large-file functions need explicit mappings for MSVC (not MinGW).
  if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
    cc_build
      .define("fseeko", Some("_fseeki64"))
      .define("ftello", Some("_ftelli64"));
  }

  if let Some((zlib_include_dir, mman_include_dir)) = &windows_includes {
    cc_build.include(zlib_include_dir).include(mman_include_dir);
  }

  cc_build.compile("fst");

  // Rebuild if C source changes.
  println!("cargo:rerun-if-changed=csrc");
  println!("cargo:rerun-if-changed=vendor/libfst/src");

  // Link with zlib.
  if !is_windows {
    println!("cargo:rustc-link-lib=z");
  }
  if is_unix {
    println!("cargo:rustc-link-lib=pthread");
  }

  // Generate bindings.
  let mut bindgen_builder = bindgen::Builder::default()
    .header("csrc/fst_tools.h")
    .allowlist_type(r#"(fst|FST_)\w+"#)
    .allowlist_function(r#"(fst|FST_)\w+"#)
    .allowlist_var(r#"(fst|FST_)\w+"#)
    .clang_arg("-Icsrc")
    .clang_arg("-Ivendor/libfst/src");

  if let Some((zlib_include_dir, _)) = &windows_includes {
    bindgen_builder = bindgen_builder.clang_arg(format!("-I{}", zlib_include_dir.display()));
  }

  let bindings = bindgen_builder
    .generate()
    .expect("failed to generate bindings");

  // Write the bindings to file.
  let out_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
  bindings
    .write_to_file(out_path)
    .expect("failed to write bindings");
}
