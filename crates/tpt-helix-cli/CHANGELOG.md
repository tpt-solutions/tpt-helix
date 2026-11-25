# Changelog

All notable changes to `tpt-helix-cli` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Initial `helix` binary with six commands: `run`, `render`, `migrate`, `build`,
  `new`, and `doctor`.
- `render` writes a PNG with the static pipeline. `--css` adds a stylesheet
  after the document's own `<style>` blocks. Viewports are limited to 8192
  pixels per side.
- `run` is deny-by-default. `--grant` takes `dom`, `storage`, `network`, and
  `media`. `--unrestricted` bypasses the capability broker.
- `migrate --report` prints the migration report, `--out` writes the guest
  crate, and `--strict` fails on warnings.
- `doctor` reports the toolchain status (cargo, rustc, the wasm32 target,
  `wasm-tools`, and `patch` on Windows) with a remediation hint for each failure.
- A library API (`tpt_helix_cli`) so the commands can be tested and reused.
- `tests/e2e.rs` runs a migrated click handler end to end: it migrates, builds,
  componentizes, and runs the guest in the runtime, then checks its storage write.
  The test is `#[ignore]`d because it needs the wasm32 target and `wasm-tools`. CI's
  `wasm-guest` job installs `wasm-tools` and runs it.

### Changed
- `migrate` prints the JavaScript (S2) outcome and reviews translated functions that need Rust types.
- Package renamed to `tpt-helix-cli`. The binary is still named `helix`.
- `migrate` reports unsupported documents instead of failing. `--out` refuses to
  write a guest crate for them, and exits non-zero.
