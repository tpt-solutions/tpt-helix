# Changelog

All notable changes to `tpt-helix-migrate` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- **Breaking:** `migrate()` returns `Result<(OrchestratedMigration, GuestCrate),
  MigrationError>`. It returns `MigrationError::Unsupported` when no pattern
  matches. The helper `generate_guest()` builds the guest crate from an
  orchestrated migration.
- Renamed the package from `helix-migrate` to `tpt-helix-migrate`. Rust imports
  change from `helix_migrate::` to `tpt_helix_migrate::`.
- Added crate metadata (description, readme, keywords, categories).

### Added
- Stage S2 translates the supplied JS/TS source with `transpile_js_to_rust`. The
  result is `OrchestratedMigration::js`, and a failure is `js_error`. Overlapping
  rewrites are skipped, and `Auto` falls back from JavaScript to TypeScript.
- `OrchestratedMigration::warnings()` is the single list of review conditions. It
  now includes the JS outcome.

### Fixed
- The JS rewriter no longer splices overlapping edits, and syntax errors are
  reported instead of being rewritten. `console.log` becomes a `println!` with a
  format string, `var` becomes `let mut`, and single-quoted strings become
  double-quoted.
- Text no longer loses its spaces. Whitespace runs collapse to one space and the
  edge spaces around inline elements are kept.
- Character references (`&amp;`, `&lt;`, `&#65;`, `&#x42;`, `&nbsp;`) are decoded in
  text and attribute values.
- `<script>` and `<style>` bodies are raw text. They are not parsed as markup and
  are never emitted as page text.
- A document with no elements is no longer classified as a strong static site. It
  is `Fit::None`, and the transpile stage is skipped for it.
- Elements with no text are a partial static-site match, not a strong one.
- Discovery (S1) reads the inline `<script>` bodies with the AST frontend, instead
  of running the line-based scanner over the whole HTML document.

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
