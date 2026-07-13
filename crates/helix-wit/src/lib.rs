//! Generated bindings for the Helix Runtime capability interfaces.
//!
//! `wit_bindgen::generate!` is run twice here, once per world in `wit/helix.wit`:
//!
//! * [`helix`] (crate root) — the **guest** world. A guest component *imports*
//!   these capabilities, so `wit-bindgen` emits free functions it calls, e.g.
//!   `helix::runtime::network::fetch`. See `crates/helix-guest-example`.
//!
//! * [`host`] (submodule) — the **host** world. The Helix Runtime *exports*
//!   the same interfaces to guests, so `wit-bindgen` emits the `Guest` traits
//!   the runtime implements to provide each capability (our generated "Host"
//!   traits). This avoids the wasmtime-dependent host generator while keeping
//!   the host contract generated from the same WIT source of truth.
//!
//! Because the package is `helix:runtime`, generated modules live under
//! `helix::runtime` (package ID => module path).

// Guest bindings: a guest component imports these capabilities.
wit_bindgen::generate!({
    world: "helix-guest",
    path: "wit",
    additional_derives: [PartialEq, Eq],
});

// Host bindings: the runtime implements these traits to provide the
// capabilities. `type_section_suffix` keeps this world's embedded WIT
// custom section distinct from the guest world's above.
pub mod host {
    wit_bindgen::generate!({
        world: "helix-host",
        path: "wit",
        type_section_suffix: "host",
        additional_derives: [PartialEq, Eq],
    });
}
