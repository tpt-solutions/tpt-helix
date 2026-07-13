//! Example guest module for the Helix Runtime.
//!
//! This crate is the guest-side counterpart to `crates/helix-runtime`: it
//! generates bindings for the `helix-guest` world (which *imports* the
//! `network`, `storage`, and `dom` capability interfaces and *exports* `run`),
//! and calls them as a real WASM component would.
//!
//! It targets `wasm32-unknown-unknown` and is componentized (see
//! `crates/helix-runtime/build.rs`) into the smoke-test component the host
//! loads through wasmtime.
//!
//! Build + componentize:
//!   cargo build -p helix-guest-example --target wasm32-unknown-unknown
//!   wasm-tools component new \
//!     target/wasm32-unknown-unknown/debug/helix_guest_example.wasm \
//!     -o hello_dom.wasm

wit_bindgen::generate!({
    world: "helix-guest",
    path: "../helix-wit/wit",
    additional_derives: [PartialEq, Eq, Hash],
});

/// Entry point the host invokes after instantiating the component.
///
/// Exercises the imported capability interfaces: builds a tiny DOM tree,
/// persists a value, and issues a fetch.
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    use helix::runtime::{dom, network, storage};

    let root = dom::create_element(&"div");
    let child = dom::create_element(&"span");
    dom::set_text(child, &"hello from guest");
    dom::set_attribute(root, &"class", &"greeting");
    dom::append_child(root, child);
    dom::on_click(root, 1);

    storage::set(&"seen", b"yes".as_slice()).expect("store");

    let _ = network::fetch(&network::Request {
        method: "GET".to_string(),
        url: "https://example.com/".to_string(),
        headers: vec![],
        body: None,
    });
}
