use std::{env, fs, path::Path, path::PathBuf};

fn main() {
  let upstream = PathBuf::from("third_party/libfst/src");
  assert!(
    upstream.join("fstapi.h").is_file(),
    "libfst sources are missing; run `git submodule update --init --recursive`"
  );
  let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
  let fst_source = patched_fst_source(&upstream, &out_dir);

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
    .files(["fastlz.c", "lz4.c"].map(|file| upstream.join(file)))
    .file(fst_source)
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
  println!("cargo:rerun-if-changed=third_party/libfst/src");
  println!("cargo:rerun-if-changed=patches");

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
    .clang_arg("-Ithird_party/libfst/src");

  if let Some((zlib_include_dir, _)) = &windows_includes {
    bindgen_builder = bindgen_builder.clang_arg(format!("-I{}", zlib_include_dir.display()));
  }

  let bindings = bindgen_builder
    .generate()
    .expect("failed to generate bindings");

  // Write the bindings to file.
  let out_path = out_dir.join("bindings.rs");
  bindings
    .write_to_file(out_path)
    .expect("failed to write bindings");
}

/// Apply the reviewed compatibility corrections to a build-local copy.
/// An upstream update must explicitly review/remove any correction whose
/// context changes; the checked-in submodule is never modified.
fn patched_fst_source(upstream: &Path, out_dir: &Path) -> PathBuf {
  let mut source = fs::read_to_string(upstream.join("fstapi.c"))
    .expect("failed to read libfst source")
    .replace("\r\n", "\n");
  let patches = [
    (
      "real-alias.patch",
      include_str!("patches/real-alias.patch"),
      1,
    ),
    (
      "msvc-varint.patch",
      include_str!("patches/msvc-varint.patch"),
      3,
    ),
    (
      "windows-stdio.patch",
      include_str!("patches/windows-stdio.patch"),
      22,
    ),
    (
      "reader-cancel.patch",
      include_str!("patches/reader-cancel.patch"),
      8,
    ),
  ];
  for (name, contents, expected_hunks) in patches {
    let contents = contents.replace("\r\n", "\n");
    let patch = diffy::Patch::from_str(&contents)
      .unwrap_or_else(|error| panic!("invalid libfst patch {name}: {error}"));
    assert_eq!(
      patch.hunks().len(),
      expected_hunks,
      "libfst patch {name} must contain exactly {expected_hunks} hunks"
    );
    source = diffy::apply(&source, &patch).unwrap_or_else(|error| {
      panic!(
        "failed to apply libfst patch {name}: {error}; review the upstream pin and patches/{name}"
      )
    });
  }
  let path = out_dir.join("fstapi.c");
  fs::write(&path, source).expect("failed to write corrected libfst source");
  path
}
