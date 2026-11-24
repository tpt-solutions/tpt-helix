# Changelog

All notable changes to `tpt-helix-wit` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Renamed the package from `helix-wit` to `tpt-helix-wit`. The WIT package
  `helix:runtime@0.1.0` is unchanged.
- Added crate metadata (description, readme, keywords, categories).

## [0.1.0]

Initial development release (pre-release, not published).

### Added
- `wit/helix.wit`, package `helix:runtime@0.1.0`, with the `network`, `storage`,
  `dom`, and `media` interfaces.
- The `helix-guest` world (imports the four interfaces, exports `run`) and the
  `helix-host` world (exports the four interfaces).
- Generated guest bindings (`helix::runtime::*`) and host trait bindings
  (`host::exports::helix::runtime::*`) via `wit-bindgen`.
