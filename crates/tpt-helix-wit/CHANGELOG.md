# Changelog

All notable changes to `tpt-helix-wit` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `helix-guest` exports `dispatch(handler-id: u64)`, which the host calls for each
  handler a DOM event triggers.
- `dom.on-event(el, event, handler-id)`, a generic DOM event registration for
  `submit`, `change`, `input`, `play`, `pause`, and `ended`. Event names are
  lowercase ASCII, up to 32 bytes.

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
