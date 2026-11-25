# Getting started with TPT Helix

Helix runs web apps as WebAssembly guests. A guest declares what it needs, and
the runtime grants only that. This page takes you from a clone to a rendered page,
a migrated app, and a running guest.

## 1. Install the toolchain

`rust-toolchain.toml` pins Rust 1.96.1 and the `wasm32-unknown-unknown` target, so
`rustup` installs them on the first `cargo` call in the repository.

Check the rest with the CLI:

```sh
cargo run -p tpt-helix-cli -- doctor
```

`doctor` reports whether `patch` (Windows), `wasm-tools`, and the wasm target are
present, with a hint for each missing one.

## 2. Render a page (no GPU needed)

```sh
cargo run -p tpt-helix-cli -- render examples/hello-static/index.html -o hello.png
```

`render` runs the static pipeline (HTML, CSS, layout, text, display list) and
writes a PNG. It works headless, so it is the quickest way to check a page.

## 3. Start a new app

```sh
cargo run -p tpt-helix-cli -- new my-page                       # a static HTML page
cargo run -p tpt-helix-cli -- new my-guest --template basic     # a guest crate
```

Guest templates (`basic`, `network`, `storage`) are described in
[guest-authoring.md](guest-authoring.md) and each ships a `capabilities` file (see
[capability-manifest.md](capability-manifest.md)).

## 4. Build and run a guest

```sh
cargo run -p tpt-helix-cli -- build my-guest
```

`build` compiles the crate for `wasm32-unknown-unknown`. The runtime loads WebAssembly
components, so componentize the output with `wasm-tools` first:

```sh
cargo install wasm-tools --version 1.263.0 --locked
wasm-tools component new my-guest/target/wasm32-unknown-unknown/release/my_guest.wasm \
  -o my-guest.component.wasm
```

Then run it with the grants from its manifest:

```sh
cargo run -p tpt-helix-cli -- run my-guest.component.wasm \
  --grant "$(cat my-guest/capabilities)" --app my-guest
```

Without `--grant`, the guest has no capabilities and every host call is denied.
`--unrestricted` bypasses the broker entirely. Use it only for local debugging.

## 5. Migrate an existing page

```sh
cargo run -p tpt-helix-cli -- migrate examples/legacy-js-app/before/index.html \
  --js examples/legacy-js-app/before/app.js --name legacy-js-app --report
```

The migration runs stages S1 to S5 and prints a report with the detected pattern,
the plan, and any warnings. Add `--out <dir>` to write the generated guest crate,
and `--strict` to fail on any warning. The [migration guide](migration-guide.md)
lists what translates and what does not.

## Where to go next

- [Guest authoring](guest-authoring.md): the guest world, exports, and the DOM API.
- [Capability manifest reference](capability-manifest.md): grants and their scope.
- [Migration guide](migration-guide.md): patterns, coverage, and limits.
- `spec.txt` is the design. `TODO.md` is the execution checklist.
