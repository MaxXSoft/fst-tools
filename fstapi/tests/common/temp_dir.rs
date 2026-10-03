use std::path::PathBuf;
use tempfile::TempDir;

/// An isolated directory beside the test executable, removed on drop.
pub struct TestDir(TempDir);

impl TestDir {
  /// Creates a unique directory within Cargo's build artifacts.
  pub fn new() -> Self {
    let test_binary = std::env::current_exe().unwrap();
    Self(tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap())
  }

  /// Returns the path of a named file within this test directory.
  pub fn path(&self, name: &str) -> PathBuf {
    self.0.path().join(name)
  }
}
