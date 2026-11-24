//! End-to-end smoke test: load a real WASM component through wasmtime, wire the
//! generated WIT host-import table, instantiate it, run its `run` entry point,
//! and assert the guest's `dom`/`storage` calls landed in the host state.
//!
//! The guest (`crates/tpt-helix-guest-example`) is pre-built and componentized into
//! `hello_dom.wasm` (see `crates/tpt-helix-runtime/build.rs` / the crate README
//! note). It builds a tiny DOM tree, stores a value, and issues a fetch.

use tpt_helix_runtime::capability::{Capability, DomScope, HostPattern, StorageScope};
use tpt_helix_runtime::wasm::bindings::helix::runtime::dom;
use tpt_helix_runtime::wasm::{Engine, Host, Module, ResourceLimits, sandboxed_engine};

/// The guest needs dom (create/append/click), storage (`seen`), and network
/// (its fetch is answered by the host, or fails cleanly without a route).
fn granted_host() -> Host {
    Host::with_grants(
        "hello-dom",
        [
            Capability::Dom {
                scope: DomScope::Full,
            },
            Capability::Storage {
                scope: StorageScope::Global,
            },
            Capability::Network {
                hosts: vec![HostPattern::Any],
            },
        ],
    )
}

#[test]
fn loads_instantiates_and_runs_hello_dom() {
    let bytes = include_bytes!("hello_dom.wasm");
    assert!(!bytes.is_empty());

    let engine = sandboxed_engine().expect("sandboxed engine");

    // Lifecycle step 1+2: load (validate + JIT-compile) and instantiate,
    // wiring the generated WIT imports into the host-import table.
    let module = Module::load(&engine, bytes).expect("component loads");
    assert!(module.source_len() > 0);

    let mut instance = module
        .instantiate(&engine, granted_host())
        .expect("component instantiates");

    // Lifecycle step 3: drive the module's exported entry point.
    instance.run().expect("module runs");

    // The run was timed as one `wasm.call` op on the instance profiler.
    let call = instance
        .profiler()
        .channel("wasm.call")
        .expect("wasm.call recorded");
    assert_eq!(call.ops, 1);

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

#[test]
fn ungranted_guest_is_denied_all_dom_and_storage() {
    let engine = sandboxed_engine().expect("sandboxed engine");
    let module = Module::load(&engine, include_bytes!("hello_dom.wasm")).expect("loads");
    let mut instance = module
        .instantiate(&engine, Host::new())
        .expect("instantiates");
    // Every capability-protected call is denied. The guest may trap if it
    // unwraps a denied result; either way no protected state may be written.
    let _ = instance.run();
    assert!(instance.host().element(dom::ElementId { id: 0 }).is_none());
    assert_eq!(instance.host().stored("seen"), None);
}

#[test]
fn fuel_exhaustion_traps_the_guest() {
    let engine = sandboxed_engine().expect("sandboxed engine");
    let module = Module::load(&engine, include_bytes!("hello_dom.wasm")).expect("loads");
    let limits = ResourceLimits {
        fuel_per_call: 1,
        ..ResourceLimits::default()
    };
    // One unit of fuel cannot cover instantiation plus the guest's run.
    let result = module
        .instantiate(&engine, granted_host().with_limits(limits))
        .and_then(|mut i| i.run());
    assert!(result.is_err());
}

#[test]
fn engine_without_sandbox_hooks_cannot_instantiate() {
    let engine = Engine::default();
    let module = Module::load(&engine, include_bytes!("hello_dom.wasm")).expect("loads");
    assert!(module.instantiate(&engine, Host::new()).is_err());
}
