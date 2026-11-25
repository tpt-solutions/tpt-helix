//! End-to-end event dispatch: the guest registers DOM handlers in `run`, and the
//! host fires events that call back into the guest's exported `dispatch`.
//!
//! The guest is `crates/tpt-helix-guest-example`, componentized into
//! `hello_dom.wasm`. Its `run` registers handler 1 as a click on the root element,
//! and handler 2 as its `submit` event. `dispatch` records which handler ran by
//! writing a storage key, so the host can observe every callback.

use tpt_helix_runtime::capability::{Capability, DomScope, HostPattern, StorageScope};
use tpt_helix_runtime::wasm::{Host, Module, sandboxed_engine};

const HELLO_DOM: &[u8] = include_bytes!("hello_dom.wasm");

/// The root element is the first element `run` creates, so it has id 0.
const ROOT: u64 = 0;

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

/// Instantiates the guest and runs its `run` entry point, which registers the
/// handlers. Returns the live instance.
fn started(host: Host) -> tpt_helix_runtime::wasm::Instance {
    let engine = sandboxed_engine().expect("sandboxed engine");
    let module = Module::load(&engine, HELLO_DOM).expect("component loads");
    let mut instance = module.instantiate(&engine, host).expect("instantiates");
    instance.run().expect("run registers the handlers");
    instance
}

#[test]
fn run_registers_the_click_and_submit_handlers() {
    let instance = started(granted_host());
    assert_eq!(instance.host().state.handler_ids(ROOT, "click"), vec![1]);
    assert_eq!(instance.host().state.handler_ids(ROOT, "submit"), vec![2]);
}

#[test]
fn firing_submit_dispatches_the_guest_handler() {
    let mut instance = started(granted_host());
    assert_eq!(instance.host().stored("submitted"), None);

    let ran = instance
        .fire_event(ROOT, "submit")
        .expect("dispatch succeeds");
    assert_eq!(ran, 1);
    assert_eq!(instance.host().stored("submitted"), Some(b"yes".to_vec()));
    // The click handler did not run for a submit.
    assert_eq!(instance.host().stored("clicked"), None);
}

#[test]
fn firing_click_dispatches_handler_one() {
    let mut instance = started(granted_host());
    assert_eq!(instance.fire_event(ROOT, "click").expect("dispatch"), 1);
    assert_eq!(instance.host().stored("clicked"), Some(b"yes".to_vec()));
}

#[test]
fn an_event_with_no_handlers_runs_nothing() {
    let mut instance = started(granted_host());
    assert_eq!(instance.fire_event(ROOT, "change").expect("dispatch"), 0);
    assert_eq!(instance.host().stored("submitted"), None);
    assert_eq!(instance.host().stored("clicked"), None);
}

#[test]
fn a_denied_storage_write_traps_the_guest_instead_of_passing_silently() {
    // The example guest unwraps its storage writes. Without a storage grant the
    // write is refused, and the unwrap traps the guest. The trap surfaces from
    // `run`, so no handler is ever registered.
    let host = Host::with_grants(
        "hello-dom",
        [Capability::Dom {
            scope: DomScope::Full,
        }],
    );
    let engine = sandboxed_engine().expect("sandboxed engine");
    let module = Module::load(&engine, HELLO_DOM).expect("component loads");
    let mut instance = module.instantiate(&engine, host).expect("instantiates");
    assert!(instance.run().is_err(), "a denied storage write must trap");
}
