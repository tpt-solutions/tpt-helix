//! wasmtime integration: JIT-compiled WASM component execution.
//!
//! This module wires the generated WIT host-import table (the `helix-guest`
//! world's `network`/`storage`/`dom` interfaces) into a wasmtime `Linker` and
//! manages the load → instantiate → run → teardown lifecycle of a guest module.
//!
//! The capability behavior lives in [`crate::stub::RuntimeState`]; [`Host`]
//! holds one by value and implements the `Guest` trait that `wasmtime`'s
//! `bindgen!` generates for the host side of the `helix-guest` world.

use wasmtime::component::{Component, Linker};
use wasmtime::Store;

/// Re-exported so consumers (and tests) can build an [`Engine`] without taking
/// a direct dependency on `wasmtime`.
pub use wasmtime::Engine;

use crate::stub::{Element, RuntimeState};

/// Generated host bindings for the `helix-guest` world.
///
/// `bindgen!` emits:
/// * the `Guest` trait the host implements to satisfy the guest's imports,
/// * `add_to_linker` to register those imports on a `Linker`,
/// * a typed `HelixGuest` wrapper used to call the guest's exported `run`.
pub mod bindings {
    wasmtime::component::bindgen!({
        world: "helix-guest",
        path: "../helix-wit/wit",
    });
}

use bindings::helix::runtime::{dom, network, storage};
use bindings::helix::runtime::{dom::Host as DomHost, network::Host as NetworkHost, storage::Host as StorageHost};
use bindings::HelixGuest;

/// Host state handed to a guest component. Implements the generated `Guest`
/// trait by delegating to a [`RuntimeState`].
pub struct Host {
    pub state: RuntimeState,
}

impl Host {
    pub fn new() -> Self {
        Host {
            state: RuntimeState::default(),
        }
    }

    /// Convenience accessor mirroring `RuntimeStub::element`.
    pub fn element(&self, id: dom::ElementId) -> Option<Element> {
        self.state.element(id)
    }

    pub fn click_count(&self, id: dom::ElementId) -> usize {
        self.state.click_count(id)
    }

    pub fn click_handler_ids(&self, id: dom::ElementId) -> Option<Vec<u64>> {
        self.state.click_handler_ids(id)
    }

    /// Convenience accessor mirroring `RuntimeStub::get`.
    pub fn stored(&self, key: &str) -> Option<Vec<u8>> {
        self.state.stored(key)
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl wasmtime::component::HasData for Host {
    type Data<'a> = &'a mut Host;
}

impl DomHost for Host {
    fn create_element(&mut self, tag: String) -> dom::ElementId {
        self.state.create_element(tag)
    }

    fn set_text(&mut self, el: dom::ElementId, text: String) {
        self.state.set_text(el, text);
    }

    fn set_attribute(&mut self, el: dom::ElementId, name: String, value: String) {
        self.state.set_attribute(el, name, value);
    }

    fn append_child(&mut self, parent: dom::ElementId, child: dom::ElementId) {
        self.state.append_child(parent, child);
    }

    fn on_click(&mut self, el: dom::ElementId, handler_id: u64) {
        self.state.on_click(el, handler_id);
    }
}

impl NetworkHost for Host {
    fn fetch(&mut self, req: network::Request) -> Result<network::Response, String> {
        self.state.fetch(req)
    }
}

impl StorageHost for Host {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.state.get(key)
    }

    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.state.set(key, value)
    }

    fn delete(&mut self, key: String) -> Result<(), String> {
        self.state.delete(key)
    }
}

/// A guest module compiled and ready to instantiate.
///
/// Wraps a wasmtime [`Component`] (the JIT-compiled artifact). Cloning is
/// cheap; the underlying compiled code is reference-counted by wasmtime.
pub struct Module {
    component: Component,
    source_len: usize,
}

impl Module {
    /// Loads and validates `bytes` as a WASM component, compiling it for JIT
    /// execution on `engine`.
    pub fn load(engine: &Engine, bytes: &[u8]) -> Result<Self, wasmtime::Error> {
        let component = Component::new(engine, bytes)?;
        Ok(Module {
            component,
            source_len: bytes.len(),
        })
    }

    /// Number of bytes in the source the module was loaded from.
    pub fn source_len(&self) -> usize {
        self.source_len
    }

    /// Instantiates the module against `host`, wiring the generated WIT imports
    /// into the host-import table. Consumes `host`, which can be recovered via
    /// [`Instance::host`] / [`Instance::host_mut`] after the call.
    pub fn instantiate(&self, engine: &Engine, host: Host) -> Result<Instance, wasmtime::Error> {
        let mut store = Store::new(engine, host);
        let mut linker: Linker<Host> = Linker::new(engine);
        bindings::helix::runtime::dom::add_to_linker(&mut linker, |h: &mut Host| h)?;
        bindings::helix::runtime::network::add_to_linker(&mut linker, |h: &mut Host| h)?;
        bindings::helix::runtime::storage::add_to_linker(&mut linker, |h: &mut Host| h)?;
        let bindings = HelixGuest::instantiate(&mut store, &self.component, &linker)?;
        Ok(Instance { bindings, store })
    }
}

/// A live guest instance.
///
/// Teardown is implicit: dropping the `Instance` (and its [`Store`]) releases
/// the guest's linear memory and host handle. Call [`Instance::run`] to drive
/// the module's exported `run` entry point.
pub struct Instance {
    bindings: HelixGuest,
    store: Store<Host>,
}

impl Instance {
    /// Invokes the guest's exported `run` entry point.
    pub fn run(&mut self) -> Result<(), wasmtime::Error> {
        self.bindings.call_run(&mut self.store)?;
        Ok(())
    }

    /// Borrows the host state after (or between) guest calls.
    pub fn host(&self) -> &Host {
        self.store.data()
    }

    pub fn host_mut(&mut self) -> &mut Host {
        self.store.data_mut()
    }
}
