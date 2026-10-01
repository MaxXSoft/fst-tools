# Changelog

## Unreleased

### Changed

* Replace the modified C snapshot with the official libfst submodule, initially
  pinned to `c716c1f67e21a06aaf8722830b371516e70dc657`. Keep Rust adaptation and
  VCD export glue separate from upstream; include the C sources in crate packages.
* Treat value emissions before an explicit timestamp as time-zero changes.

### Fixed

* Reject invalid writer operations without corrupting initial-value timestamps;
  preserve static initial values and initial variable-length values.
* Keep hierarchy records valid as iteration advances, and resume callback panics
  only after returning from the C traversal.
* Include value changes in VCD exports and report stream/close failures.
* Preserve real, EVCD port, and string values when clipping, including initial
  values for windows with no internal changes, and preserve the input timezero.
* Display SystemVerilog array scopes in `readfst`.

### Added

* Crate and command-line integration regressions, SV array scope constant, and
  CI verification of packaged source builds.

## 0.0.3 - 2025-10-22

### Added

* [PR #4](https://github.com/MaxXSoft/fst-tools/pull/4): Windows support using vcpkg [x64-windows-static-md].

### Changed

* Update to Rust 2024 edition.

## 0.0.2 - 2024-04-14

### Fixed

* [PR #1](https://github.com/MaxXSoft/fst-tools/pull/1): fix(utils): support more platforms.

## 0.0.1
