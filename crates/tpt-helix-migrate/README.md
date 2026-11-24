# tpt-helix-migrate

The migration pipeline for TPT Helix. It takes an existing HTML and JavaScript
app and produces a componentizable Rust guest crate that runs on the Helix
runtime, through five stages (spec §6):

| Stage | Name | What happens | Module |
| --- | --- | --- | --- |
| S1 | Discovery | Read the JS/TS API surface: imports, exports, top-level functions. | `tree_sitter_discovery`, `lib::discover` |
| S2 | Transpile | Classify the document into a pattern and emit DOM operations plus Rust source. | `transpile`, `js_transform`, `type_infer`, `coverage` |
| S3 | Validate | Check the transpiled output for equivalence. | `eve::validate_equivalence`, `tests/equivalence.rs` |
| S4 | Optimize | Suggest dead-code removal, struct packing, and parallel loops. | `optimize` |
| S5 | Deploy | Emit `helix-deploy.toml` and a feature-flag module. | `deploy` |

`eve` plans and runs the stages. `spark` folds their output into a guest crate
(`Cargo.toml`, `src/lib.rs`, and a WIT world).

## Patterns

Eve classifies each document into one of six patterns. Each pattern has its own
transpiler and its own support probe.

| Pattern | Shape | Transpiler |
| --- | --- | --- |
| P1 | Static content site | `transpile_static_site` |
| P2 | Form and CRUD app (forms, entity tables) | `transpile_form_app` |
| P3 | Data visualization (`canvas` or `svg` charts) | `transpile_data_viz` |
| P4 | Media player (`video` or `audio`) | `transpile_media_player` |
| P5 | Real-time collaboration (`data-realtime`) | `transpile_realtime` |
| P6 | SPA shell with routes | `transpile_spa` |

## Usage

Through the CLI:

```sh
helix migrate app.html --report                 # print the migration report
helix migrate app.html --js app.js --out ./out  # also write the guest crate
helix migrate app.html --report --strict        # exit non-zero on any warning
```

From Rust:

```rust
use tpt_helix_migrate::{eve::PlanRequest, migrate};
use tpt_helix_migrate::deploy::TargetPlatform;

let (migration, guest) = migrate(&PlanRequest {
    app_name: "notes".into(),
    version: "0.1.0".into(),
    html: std::fs::read_to_string("notes.html")?,
    js: None,
    targets: vec![TargetPlatform::Wasm32],
});
println!("pattern: {:?}, equivalence: {}", migration.plan.detected, migration.equivalence_ok);
std::fs::write("guest/src/lib.rs", guest.lib_rs)?;
```

## Status and known gaps

This crate is under active development, and the transpiler is **not complete**.
The open items are tracked in the workspace `TODO.md` under "Review
Follow-ups". The main ones:

- **Pattern detection defaults to P1 with strong confidence.** A document with
  no recognizable structure is reported as a supported static site. The CLI
  therefore cannot tell "static site" from "not understood".
- **The JS transpiler is not wired into S2.** `js_transform::transpile_js_to_rust`
  exists but nothing calls it. Its output also needs fixes (valid `println!`,
  parameter types, overlapping edits).
- **The HTML parser is hand-written.** It does not treat `<script>` and `<style>`
  content as raw text, and it does not decode character entities.
- **Equivalence (S3) is structural.** It checks that every append refers to a
  created node and that some text survived. It does not compare rendered output.
- **Discovery (S1) for HTML uses the line-based tokenizer** on the whole
  document. It should read inline `<script>` blocks through the AST parser.
- **`migrate()` does not stop on a failed stage.** `Fit::None` and a failed
  equivalence check produce warnings, not errors.

Treat a migration as a draft until its report has no warnings and its output has
been reviewed by hand.

## Testing

```sh
cargo test -p tpt-helix-migrate
```

The discovery and equivalence suites live in `tests/discovery.rs` and
`tests/equivalence.rs`. The P1 support probe runs in `coverage::Pattern::supported`.

## License

MIT OR Apache-2.0. See the workspace `LICENSE-MIT` and `LICENSE-APACHE`.
