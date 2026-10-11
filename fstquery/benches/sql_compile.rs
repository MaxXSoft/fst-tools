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

use criterion::{Criterion, criterion_group, criterion_main};
use std::time::Duration;

criterion_group! {
  name = benches;
  config = Criterion::default()
    .warm_up_time(Duration::from_millis(100))
    .measurement_time(Duration::from_millis(500))
    .sample_size(50);
  targets = sql::bench::benchmarks
}
criterion_main!(benches);
