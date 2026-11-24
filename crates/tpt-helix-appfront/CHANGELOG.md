# Changelog

All notable changes to `tpt-helix-appfront` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Package renamed to `tpt-helix-appfront`. Rust imports change from `appfront::`
  to `tpt_helix_appfront::`. The binary is still named `appfront`.
- `egui` is now optional and enabled by the `window` feature, so the headless
  build no longer pulls in the GUI stack.
- Added crate metadata and a README.

## [0.1.0]

Initial development release (pre-release, not published).

### Added
- `HelixDocument`: parses HTML and computes a `taffy` layout tree.
- CSS parsing and per-element property resolution, including `<style>` block
  extraction and color parsing.
- A flat render list (`RectItem`, `TextItem`) for painting.
- The `window` feature, which opens a native `eframe` window that paints the
  render list with `egui`.
