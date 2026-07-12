//! Example guest module for the Helix Runtime.
//!
//! This crate is the guest-side counterpart to `crates/helix-runtime`: it
//! generates bindings for the `helix-guest` world (which *imports* the
//! `network`, `storage`, and `dom` capability interfaces) and calls them as a
//! real WASM module would. It targets `wasm32-unknown-unknown` and is wired
//! into a component `Linker` by the host once `wasmtime` is integrated
//! (TODO.md §"Basic WASM module loading").
//!
//! Build with:
//!   cargo build -p helix-guest-example --target wasm32-unknown-unknown

wit_bindgen::generate!({
    world: "helix-guest",
    path: "../helix-wit/wit",
    additional_derives: [PartialEq, Eq, Hash],
});

/// Exercises the imported capability interfaces the way a real guest would.
///
/// This is the body a host would invoke after instantiating the component;
/// here it builds a tiny DOM tree, persists a value, and issues a fetch.
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub fn run() {
    use helix::runtime::{dom, network, storage};

    let root = dom::create_element(&"div".to_string());
    let child = dom::create_element(&"span".to_string());
    dom::set_text(child, &"hello from guest".to_string());
    dom::set_attribute(root, &"class".to_string(), &"greeting".to_string());
    dom::append_child(root, child);
    dom::on_click(root, 1);

    storage::set(&"seen".to_string(), &b"yes".to_vec()).expect("store");

    let _ = network::fetch(&network::Request {
        method: "GET".to_string(),
        url: "https://example.com/".to_string(),
        headers: vec![],
        body: None,
    });
}
