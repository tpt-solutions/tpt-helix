# tpt-helix-cli

The `helix` command-line tool for TPT Helix. It runs guests, renders pages to
PNG without a display, migrates apps, builds guest crates, scaffolds projects,
and checks that your toolchain is ready.

## Install

From the workspace root:

```sh
cargo install --path crates/tpt-helix-cli
```

This installs a binary named `helix`.

## Commands

### `helix render`

Renders an HTML document to a PNG with the same static pipeline the GPU presenter
uses. No GPU or display is needed.

```sh
helix render page.html                       # writes page.png beside the input
helix render page.html -o out.png --width 1280 --height 720
helix render page.html --css theme.css       # extra CSS, applied after <style>
```

The viewport is limited to 8192 pixels per side. The document's own `<style>`
blocks apply first, and `--css` is applied after them.

### `helix run`

Instantiates a WebAssembly component and calls its `run` export.

```sh
helix run guest.wasm --grant dom,storage
helix run guest.wasm --app notes --grant network
helix run guest.wasm --unrestricted          # bypass the capability broker
```

Deny by default: with no `--grant`, the guest can do nothing. The grant names are
`dom`, `storage`, `network`, and `media`. `media` is granted up to 1920×1080.
`--unrestricted` skips the capability broker. Use it only for code you trust.

### `helix migrate`

Runs the S1–S5 migration pipeline on an HTML document.

```sh
helix migrate app.html --report                     # print the report
helix migrate app.html --js app.js --out ./app-guest
helix migrate app.html --report --strict            # fail on any warning
```

The report lists the detected pattern, its confidence, each stage's status,
equivalence, and the warnings that need a human. `--out` writes the generated
guest crate to a new or empty directory. The command requires `--report`, `--out`,
or both.

> **Known limitation:** the transpiler is not complete. Unsupported documents
> get a report and no guest crate, and `--out` refuses to write for them. See
> `tpt-helix-migrate/README.md` for the open transpiler work.

### `helix build`

Builds a guest crate for `wasm32-unknown-unknown` in release mode.

```sh
helix build my-guest
```

### `helix new`

Creates a static starter app.

```sh
helix new hello            # creates hello/index.html
helix render hello/index.html
```

It refuses to write into an existing directory.

### `helix doctor`

Checks the toolchain the other commands depend on. It exits non-zero when a
required check fails.

```sh
helix doctor
```

| Check | Required | Needed for |
| --- | --- | --- |
| `cargo`, `rustc` | yes | `build` |
| `wasm32-unknown-unknown` target | yes | `build` |
| `wasm-tools` | no | inspecting components |
| `patch` (Windows only) | yes | building `rquickjs`, a dependency of the runtime |

## Exit status

`0` on success. `1` on any error, including `--strict` warnings and failed
`doctor` checks. Errors are printed to stderr as `error: <message>`.

## Library

The commands are plain functions in the `tpt_helix_cli` library, so tests and
other tools can call them directly.

```rust
use tpt_helix_cli::{migrate_html, migration_report};

let (migration, guest) = migrate_html("notes", "<p>hello</p>", None);
print!("{}", migration_report(&migration));
// `guest` is `None` when no pattern matched.
```

## License

MIT OR Apache-2.0. See the workspace `LICENSE-MIT` and `LICENSE-APACHE`.
