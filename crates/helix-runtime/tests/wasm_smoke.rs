//! End-to-end smoke test: load a real WASM component through wasmtime, wire the
//! generated WIT host-import table, instantiate it, run its `run` entry point,
//! and assert the guest's `dom`/`storage` calls landed in the host state.
//!
//! The guest (`crates/helix-guest-example`) is pre-built and componentized into
//! `hello_dom.wasm` (see `crates/helix-runtime/build.rs` / the crate README
//! note). It builds a tiny DOM tree, stores a value, and issues a fetch.

use helix_runtime::wasm::bindings::helix::runtime::dom;
use helix_runtime::wasm::{Engine, Host, Module};

#[test]
fn loads_instantiates_and_runs_hello_dom() {
    let bytes = include_bytes!("hello_dom.wasm");
    assert!(!bytes.is_empty());

    let engine = Engine::default();

    // Lifecycle step 1+2: load (validate + JIT-compile) and instantiate,
    // wiring the generated WIT imports into the host-import table.
    let module = Module::load(&engine, bytes).expect("component loads");
    assert!(module.source_len() > 0);

    let mut instance = module
        .instantiate(&engine, Host::new())
        .expect("component instantiates");

    // Lifecycle step 3: drive the module's exported entry point.
    instance.run().expect("module runs");

    // Inspect host state produced by the guest's capability calls.
    let host = instance.host();
    let root = dom::ElementId { id: 0 };
    let child = dom::ElementId { id: 1 };

    let root_el = host.element(root).expect("root element exists");
    assert_eq!(root_el.tag, "div");
    assert_eq!(
        root_el.attributes.get("class").map(String::as_str),
        Some("greeting")
    );
    assert_eq!(root_el.children.len(), 1);
    assert_eq!(root_el.children[0].id, 1);

    let child_el = host.element(child).expect("child element exists");
    assert_eq!(child_el.tag, "span");
    assert_eq!(child_el.text, "hello from guest");

    // dom.on_click registered handler id 1 against the root.
    assert_eq!(host.click_handler_ids(root), Some(vec![1u64]));

    // storage.set("seen", b"yes") persisted.
    assert_eq!(host.stored("seen"), Some(b"yes".to_vec()));

    // Lifecycle step 4 (teardown): dropping the instance releases the guest.
    drop(instance);
}
