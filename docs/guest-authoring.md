# Guest authoring

A Helix guest is a WebAssembly component that imports the Helix capability
interfaces and exports entry points the host calls. The interfaces are defined
once, in `crates/tpt-helix-wit/wit/helix.wit`. The same text is in `spec.txt` §5.2,
and a test keeps the two identical.

## The world

A guest is built against the `helix-guest` world:

- It **imports** `network`, `storage`, `dom`, and `media`.
- It **exports** `run: func()`. The host calls it once after instantiation.
- It **exports** `dispatch: func(handler-id: u64)`. The host calls it for each
  handler that a DOM event triggers.

Handlers are opaque ids, not function values. You register one with
`dom::on_click(el, id)` or `dom::on_event(el, "submit", id)`, and `dispatch(id)`
runs when the event fires.

## Project layout

A guest crate looks like the templates in `templates/`:

```
my-guest/
  Cargo.toml        # cdylib, wit-bindgen
  capabilities      # the grants this guest needs (see capability-manifest.md)
  wit/helix.wit     # the interfaces, copied from the canonical file
  src/lib.rs        # the guest
```

`wit/helix.wit` is in the crate, so the bindings resolve wherever the crate is
written. Generate the bindings with:

```rust
wit_bindgen::generate!({
    world: "helix-guest",
    path: "wit",
    additional_derives: [PartialEq, Eq, Hash],
});
```

## Writing a guest

```rust
use helix::runtime::{dom, storage};

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    let heading = dom::create_element(&"h1");
    dom::set_text(heading, &"Hello");
    dom::on_click(heading, 1);
    let _ = storage::set(&"seen", b"yes".as_slice());
}

#[unsafe(no_mangle)]
pub extern "C" fn dispatch(handler_id: u64) {
    let _ = handler_id; // route by id here
}
```

The `dom`, `network`, `storage`, and `media` modules are generated from the
interfaces, so the signatures match `helix.wit`. Errors come back as `Result` with a
`String` error.

## Building and componentizing

```sh
cargo build --target wasm32-unknown-unknown --release
wasm-tools component new target/wasm32-unknown-unknown/release/<crate>.wasm -o app.wasm
```

`wasm-tools` is pinned to 1.263.0 in CI. Run `helix doctor` to check it.

## Limits

The runtime enforces these caps. A guest that exceeds one is denied, not allowed to
grow host memory without bound:

- Elements: 100,000 per app.
- Click and named-event handlers: 100,000 in total, with `MAX_EVENT_NAME_BYTES` = 32
  for event names.
- Storage: 10,000 keys, 256-byte keys, 1 MiB values.
- Execution budget and memory: the tpt-wasm engine caps each call's execution steps
  and caps memory and tables per guest. There is no wall-clock deadline yet.

Do not rely on these numbers as stable. They live in
`crates/tpt-helix-runtime/src/stub.rs` and `wasm.rs`.
