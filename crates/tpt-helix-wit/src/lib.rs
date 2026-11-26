//! Generated bindings for the Helix Runtime capability interfaces.
//!
//! `wit_bindgen::generate!` is run twice here, once per world in `wit/helix.wit`:
//!
//! * [`helix`] (crate root) — the **guest** world. A guest component *imports*
//!   these capabilities, so `wit-bindgen` emits free functions it calls, e.g.
//!   `helix::runtime::network::fetch`. See `crates/tpt-helix-guest-example`.
//!
//! * [`host`] (submodule) — the **host** world. The Helix Runtime *exports*
//!   the same interfaces to guests, so `wit-bindgen` emits the `Guest` traits
//!   the runtime implements to provide each capability (our generated "Host"
//!   traits). This avoids the tpt-wasm-dependent host generator while keeping
//!   the host contract generated from the same WIT source of truth.
//!
//! Because the package is `helix:runtime`, generated modules live under
//! `helix::runtime` (package ID => module path).

/// The canonical WIT source (`wit/helix.wit`), embedded so generated guest crates
/// can ship their own copy of the interfaces they are built against.
pub const HELIX_WIT: &str = include_str!("../wit/helix.wit");

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
