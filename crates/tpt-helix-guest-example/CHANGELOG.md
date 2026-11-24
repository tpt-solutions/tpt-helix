# Changelog

All notable changes to `tpt-helix-guest-example` are recorded here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Package renamed to `tpt-helix-guest-example`. The built artifact is
  `tpt_helix_guest_example.wasm`.
- Added crate metadata and a README.

## [0.1.0]

Initial development release (not published).

### Added
- A `wasm32-unknown-unknown` guest that imports the Helix capability interfaces
  and exports `run`, used as the runtime's smoke-test component.
