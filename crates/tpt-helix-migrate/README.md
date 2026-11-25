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

// `migrate` fails with `MigrationError::Unsupported` when no pattern matches.
let (migration, guest) = migrate(&PlanRequest {
    app_name: "notes".into(),
    version: "0.1.0".into(),
    html: std::fs::read_to_string("notes.html")?,
    js: None,
    targets: vec![TargetPlatform::Wasm32],
})?;
println!("pattern: {:?}, equivalence: {}", migration.plan.detected, migration.equivalence_ok);
std::fs::write("guest/src/lib.rs", guest.lib_rs)?;
```

## Status and known gaps

This crate is under active development, and the transpiler is **not complete**.
The open items are tracked in the workspace `TODO.md` under "Review
Follow-ups".

Behavior you can rely on now:

- **Unsupported documents are refused.** A document with no elements matches no
  pattern (`Fit::None`). `migrate()` returns `MigrationError::Unsupported` for it,
  and no transpile output is produced.
- **Text keeps its words.** Whitespace runs collapse to one space, and edge spaces
  around inline elements survive. Character references (`&amp;`, `&#65;`, and so on)
  are decoded in text and attributes.
- **Script and style bodies are not page text.** `<script>` and `<style>` bodies are
  raw text. They never become visible content.
- **Discovery reads code.** With no explicit JS source, S1 parses the inline
  `<script>` bodies with the AST frontend.
- **`localStorage.setItem(key, value)` writes through the storage capability.** A
  denied write traps the guest.
- **Event handlers can call translated functions.** An attribute such as
  `onclick="greet()"` dispatches to `greet` when `greet` is parameterless, fully typed,
  and translated. Any other handler (an argument, an untyped function, an inline body)
  is reported as not wired.

Still open:

- **The JS translation is a draft.** `js_transform::transpile_js_to_rust` runs in S2
  and its result is on `OrchestratedMigration::js`. It rewrites `function`, `const`,
  `var`, `console.log`, and single-quoted strings. TypeScript annotations map to Rust
  types (`number` to `f64`, `string` to `&str`, `boolean` to `bool`, `T[]` to
  `Vec<T>`). Plain JavaScript carries no types, so its functions stay in
  `needs_rust_types`, and so does any function with an unmapped type. A literal
  integer assigned to a `number` still needs a human to fix it. A parameterless
  function whose literal return is a number or a bool is typed from that return
  (`-> i64`, `-> f64`, `-> bool`). Every translated function a handler reaches is
  emitted into the guest, with the functions it calls. Functions with parameters, and
  functions that return a string, are not typed, so they are reported and not emitted.
- **The HTML parser is hand-written.** It is not `html5ever`.
- **Equivalence (S3) is structural.** It checks that every append refers to a
  created node and that some text survived. It does not compare rendered output.
- **`discover` is AST-backed.** It runs the tree-sitter TypeScript frontend. The
  line-based scanner was removed.

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
