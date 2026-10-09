//! Standalone harness for the binary crate's private SQL implementation.

// Compile the production modules directly so benchmarking does not require a
// public library API. FST execution and CLI error helpers are unused here.
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
// Cargo enables cfg(test), but this custom harness does not run #[test] bodies;
// imports used only inside those bodies can consequently appear unused.
#[allow(dead_code, unused_imports)]
#[path = "../src/sql/mod.rs"]
mod sql;

#[path = "support/runner.rs"]
mod runner;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  runner::run()
}
