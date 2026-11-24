# Changelog

All notable changes to `tpt-helix-migrate` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Renamed the package from `helix-migrate` to `tpt-helix-migrate`. Rust imports
  change from `helix_migrate::` to `tpt_helix_migrate::`.
- Added crate metadata (description, readme, keywords, categories).

## [0.1.0]

Initial development release (pre-release, not published).

### Added
- Five-stage pipeline (S1 discovery to S5 deploy), orchestrated by `eve` and
  assembled into a guest crate by `spark`. Entry point: `migrate()`.
- Six transpilation patterns (P1 to P6) with per-pattern support probes in
  `coverage`.
- Tree-sitter-backed JS/TS discovery (`discover_ast`, `discover_js`,
  `discover_ts`, `discover_tsx`) and a line-based fallback (`discover`).
- Stage S4 suggestions: dead-code elimination, struct packing, and parallel-loop
  hints.
- Stage S5 output: `helix-deploy.toml` and a feature-flag Rust module.
- `optimize`, `type_infer`, and `js_transform` modules, with a rule-based
  JS-to-Rust rewriter.

### Known issues
- Pattern detection defaults to P1 with strong confidence when no structure is
  recognized.
- `transpile_js_to_rust` is not called from the pipeline.
- The HTML parser does not handle raw-text elements or character entities.
- Equivalence checking is structural only.
