# tpt-helix-guest-example

A minimal WebAssembly guest for TPT Helix. It is the reference for how an app
written against the Helix capability interfaces is built, and it is the smoke
test the runtime loads to check that the host and guest bindings agree.

The crate is not published (`publish = false`). It is a template and a test
fixture.

## What it does

The guest imports the `network`, `storage`, `dom`, and `media` interfaces from
`tpt-helix-wit`, and exports `run`. Its `run` entry point exercises the DOM
interface by building a small element tree.

## Build

The guest only builds for `wasm32-unknown-unknown`:

```sh
rustup target add wasm32-unknown-unknown
cargo build -p tpt-helix-guest-example --target wasm32-unknown-unknown --release
```

The output is a core module. Wrap it as a component before handing it to the
runtime:

```sh
wasm-tools component new target/wasm32-unknown-unknown/release/tpt_helix_guest_example.wasm -o guest.wasm
helix run guest.wasm --grant dom
```

`helix build` runs the first command for you on any guest crate.

## Use it as a template

Copy the crate, change the package name, and keep the bindings dependency on
`tpt-helix-wit`. The generated `helix-guest.wit` world the migration pipeline
writes uses the same interfaces.

```toml
[dependencies]
tpt-helix-wit = { path = "../tpt-helix-wit" }
```

## Notes

- A guest that is not granted a capability is denied that operation at runtime.
  The example grants nothing by default, so it does no work unless the host grants
  `dom`.
- Bindings are generated at build time, so you never edit generated code.

## License

MIT OR Apache-2.0. See the workspace `LICENSE-MIT` and `LICENSE-APACHE`.
