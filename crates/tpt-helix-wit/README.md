# tpt-helix-wit

WIT interface definitions for the TPT Helix capability model, plus the Rust
bindings generated from them for both sides of the boundary: the **guest** that
a WebAssembly component imports, and the **host** that implements it.

`wit/helix.wit` is the source of truth. The workspace spec (`spec.txt` §5.2)
describes the same interfaces; this crate is the machine-checkable version.

## The interfaces

All interfaces live in the package `helix:runtime@0.1.0`.

| Interface | Operations | Capability it needs |
| --- | --- | --- |
| `network` | `fetch(request) -> result<response, string>` | `network` (host pattern) |
| `storage` | `get`, `set`, `delete` on string keys | `storage` (namespace) |
| `dom` | `create-element`, `set-text`, `set-attribute`, `append-child`, `on-click` | `dom` (full or subtree) |
| `media` | `create-player`, `play`, `pause`, `seek` | `media` (resolution bound) |

Each operation is checked by the host's capability broker. A guest cannot reach
an operation its app was not granted.

## Worlds

- **`helix-guest`** is what a guest compiles against. It imports `network`,
  `storage`, `dom`, and `media`, and exports one entry point, `run: func()`,
  that the host calls after instantiation.
- **`helix-host`** is the host side. The runtime exports the four capability
  interfaces. `wit-bindgen` generates a `Guest` trait per interface for the
  host to implement.

## Using the bindings

Host code (`tpt-helix-runtime`) implements the generated traits. Guest code
(`tpt-helix-guest-example`) calls the generated shims:

```rust
// Guest side: imports are plain function calls.
use helix::runtime::dom;

let parent = dom::create_element("section");
let child = dom::create_element("p");
dom::set_text(child, "hello from a guest");
dom::append_child(parent, child);
```

The guest module is built for `wasm32-unknown-unknown` and then wrapped as a
component, for example with `wasm-tools component new`.

## Regenerating the bindings

The bindings are generated at build time by `wit_bindgen::generate!`, so there
is nothing to run by hand. Editing `wit/helix.wit` rebuilds both sides.

```sh
cargo build -p tpt-helix-wit
```

## Notes for contributors

- The WIT interface names are the public contract. Renaming one breaks every
  guest built against it.
- The crate is `publish = false`. Its consumers are workspace members.
- `wasmtime` comments in `helix.wit` refer to the current host implementation.
  They will need updating if the host changes.

## License

MIT OR Apache-2.0. See the workspace `LICENSE-MIT` and `LICENSE-APACHE`.
