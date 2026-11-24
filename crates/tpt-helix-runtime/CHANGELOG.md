# Changelog

All notable changes to `tpt-helix-runtime` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate follows
semantic versioning once it reaches 1.0.

## [Unreleased]

### Changed
- Renamed the package from `helix-runtime` to `tpt-helix-runtime`. Rust
  imports change from `helix_runtime::` to `tpt_helix_runtime::`.
- DASH manifests are parsed with `roxmltree` instead of a hand-written scanner.
  Malformed XML is now rejected. Previously it was tolerated.
- `init_logging()` installs a `tracing` subscriber filtered by `RUST_LOG`.

### Added
- Capability-denial audit log (target `helix::capability`) with a remediation
  hint for each denied capability.
- `inline_styles()` collects the `<style>` blocks of a parsed document.
- Per-app guest state caps: `MAX_ELEMENTS`, `MAX_CLICK_HANDLERS`,
  `MAX_STORAGE_ENTRIES`, `MAX_STORAGE_KEY_BYTES`, and `MAX_STORAGE_VALUE_BYTES`.
- JS batch cap `MAX_BATCH_OPS`. An over-cap batch throws a `RangeError` on
  commit and applies nothing.
- libp2p frame caps: `MAX_REQUEST_BYTES` and `MAX_BLOCK_BYTES`, enforced before
  allocation.
- `MAX_MPD_XML_NODES` bounds the XML node count of a DASH manifest.
- `MAX_CSS_BLOCK_DEPTH` and `MAX_SELECTOR_NESTING` limit stylesheet and selector
  nesting.

### Fixed
- Stack overflow on deeply nested selectors (for example `:not(` nested
  thousands deep). Such selectors are now rejected.
- Stack overflow on deeply nested stylesheet blocks inside `lightningcss`.
  Such stylesheets now produce no rules.
- DASH `BaseURL` nested in one `AdaptationSet` leaked into sibling sets, and
  element names matched by prefix (`<RepresentationHint>` counted as a
  `<Representation>`).
- Clippy `redundant_closure` in `benches/js_engine_compare.rs`.

## [0.1.0]

Initial development release (pre-release, not published).

### Added
- HTML parsing (`html5ever`) with charset sniffing, and CSS parsing
  (`lightningcss`, `selectors`).
- Layout with `taffy`, a display list, and a software rasterizer that produces
  PNG output without a GPU.
- Text shaping (`cosmic-text`), font loading (`fontdb`), and image and SVG
  decoding.
- WASM component hosting with fuel and epoch limits, a memory and table
  `ResourceLimiter`, and deny-by-default capability grants.
- Capability broker supporting grant, revoke, delegation, and `Subtree` and
  `Namespace` scopes.
- QuickJS interpreter with an abort-on-deadline `eval`, bridged to the DOM,
  storage, and network interfaces.
- Content-addressed storage and an in-process DHT/bitswap model, with an
  optional real libp2p transport behind the `libp2p` feature.
- DASH manifest parsing, segment planning, and ABR selection.
- Profiler spans and a `wgpu` presenter.
