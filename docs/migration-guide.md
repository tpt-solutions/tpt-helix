# Migration guide

`helix migrate` turns an HTML page (with optional JavaScript) into a Helix guest
crate. It runs the staged pipeline from `spec.txt` §6:

| Stage | What it does                                                 | Code                                    |
|-------|--------------------------------------------------------------|-----------------------------------------|
| S1    | Discovery: imports, exports, functions (tree-sitter)           | `tpt-helix-migrate/src/tree_sitter_discovery.rs` |
| S2    | Transpile: pattern detection, HTML and JS rewrites, type inference | `transpile.rs`, `js_transform.rs`, `type_infer.rs` |
| S3    | Validate: equivalence tests                                   | `tests/equivalence.rs`                  |
| S4    | Optimize: dead-code elimination, suggestions                  | `optimize.rs`                           |
| S5    | Deploy: config and feature flags                              | `deploy.rs`                             |

`eve.rs` plans the stages per app, and `spark.rs` folds the output into a guest
crate.

## Run it

```sh
helix migrate page.html --js app.js --name my-app --report
helix migrate page.html --js app.js --out my-app-guest --strict
```

`--report` prints the pattern, the plan, and every warning. `--strict` exits
non-zero when there are warnings. `--out` writes the crate, which you then build
(see [guest-authoring.md](guest-authoring.md)).

## Confidence report

`--confidence` prints four facts, with no combined score: the pattern and its fit,
the equivalence check (S3), the warning count, and a screenshot check. `--check` with
`--out` also builds the written crate for wasm32 and reports the result.

The screenshot check renders the original page and the page the guest rebuilds, at
800x600, and reports the share of pixels that differ. Both renders use the fonts
installed on the machine, so compare results from the same machine. Text does not
wrap to the width of their box. Style blocks are not carried into the guest, so a page
that depends on its stylesheet shows the lost styling as a difference.

## Patterns

Detection looks at the parsed structure of the page, not at element ids or class
names. The first match wins, in this order:

| Pattern | Matches                                                    | Example                              |
|---------|------------------------------------------------------------|--------------------------------------|
| P5 realtime   | An element with `data-realtime` and a transport          | `examples/`: none yet                |
| P6 SPA        | A root element with routes                               | `<div id="app">` with `<a href>` links |
| P4 media      | A `<video>` or `<audio>` with a source                   | `examples/video-player`              |
| P3 data viz   | A `<canvas>` with series and a size                      | `examples/chart-dashboard`           |
| P2 form CRUD  | A `<form>` or a `<table>`. Strong when a form has a submit button, named fields, and a table | `examples/todo-crud`                 |
| P1 static     | Any other page. Strong when it has text                  | `examples/hello-static`              |

A page with no elements is `Fit::None` and produces no guest. An element-only
page with no text is `Fit::Partial`, and it is reported with a warning.

## Coverage and limits

The 6 patterns each have a sample and a compile check
(`tests/guest_build.rs`, run in CI's `wasm-guest` job). Compiling does not prove
the guest behaves like the original page.

| Area                                   | Status                                                              |
|----------------------------------------|---------------------------------------------------------------------|
| Pattern detection (6 patterns)         | Implemented, structure-based                                          |
| Generated guest compiles for wasm32     | 6/6 samples, checked in CI                                            |
| Click handlers that call a named `name()` | Translated when parameterless and fully typed                        |
| Handlers with arguments                 | Not translated. Reported as a warning                                  |
| Untyped JavaScript functions            | Listed in `needs_rust_types`. Plain JS carries no types                |
| Inline handler bodies (`onclick="a(); b()"`) | Not translated. Reported as a warning                             |
| Translated JS in the guest crate        | Functions a handler reaches are emitted, transitively. Functions in `needs_rust_types` are not |
| HTML parsing                            | Hand-written parser, not `html5ever` yet                              |
| S3 screenshot equivalence               | Not built. The compile check does not prove the component links or runs |
| Partner codebases                       | None migrated yet (see `TODO.md`, Phase 2)                             |

Read the warnings from `--report` before you trust a migration. Each warning names
the construct that was left out.

## Examples

`examples/legacy-js-app/before` is a legacy page with an inline handler.
`examples/legacy-js-app/after` is the guest it produces. The CI test
`crates/tpt-helix-cli/tests/examples.rs` fails if the generator changes the output
without the committed `after/` being updated.
