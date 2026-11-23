//! wasmtime integration: JIT-compiled WASM component execution.
//!
//! This module wires the generated WIT host-import table (the `helix-guest`
//! world's `network`/`storage`/`dom` interfaces) into a wasmtime `Linker` and
//! manages the load → instantiate → run → teardown lifecycle of a guest module.
//!
//! The capability behavior lives in [`crate::stub::RuntimeState`]; [`Host`]
//! holds one by value and implements the `Host` trait that `wasmtime`'s
//! `bindgen!` generates for the imported interfaces of the `helix-guest` world.

use wasmtime::Store;
use wasmtime::component::{Component, Linker};

/// Re-exported so consumers (and tests) can build an [`Engine`] without taking
/// a direct dependency on `wasmtime`.
pub use wasmtime::Engine;

use crate::capability::{AppId, Capability, CapabilityBroker};
use crate::profiler::Profiler;
use crate::stub::{Element, Request, RuntimeState};

/// Builds a wasmtime [`Engine`] with the sandbox hooks the runtime relies on:
/// fuel metering (deterministic instruction budget) and epoch interruption
/// (wall-clock deadline driven by [`Engine::increment_epoch`]).
///
/// Engines from [`Engine::default`] have neither enabled; instantiating a
/// module on one fails, because the per-run budget cannot be applied.
pub fn sandboxed_engine() -> Result<Engine, wasmtime::Error> {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    Engine::new(&config)
}

/// Resource budget applied to every guest instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum linear memory a guest may grow to, in bytes.
    pub max_memory_bytes: usize,
    /// Maximum number of elements a guest table may grow to.
    pub max_table_elements: usize,
    /// Fuel (roughly, executed instructions) granted to each instantiation and
    /// each `run` call. Exhaustion traps the guest.
    pub fuel_per_call: u64,
    /// Epoch ticks until the deadline fires. The host advances the engine's
    /// epoch from a watchdog; reaching the deadline traps the guest.
    pub epoch_deadline_ticks: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            max_memory_bytes: 64 * 1024 * 1024,
            max_table_elements: 10_000,
            fuel_per_call: 1_000_000_000,
            epoch_deadline_ticks: 1,
        }
    }
}

impl wasmtime::ResourceLimiter for ResourceLimits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= self.max_memory_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= self.max_table_elements)
    }
}

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

use bindings::HelixGuest;
use bindings::helix::runtime::{dom, media, network};
use bindings::helix::runtime::{
    dom::Host as DomHost, media::Host as MediaHost, network::Host as NetworkHost,
    storage::Host as StorageHost,
};

/// Host state handed to a guest component. Implements the generated `Host`
/// trait by delegating to a binding-neutral [`RuntimeState`].
pub struct Host {
    pub state: RuntimeState,
    pub limits: ResourceLimits,
}

impl Host {
    /// A host with **no grants**: every capability-checked operation is denied
    /// until a grant is installed. Use [`Host::with_grants`] to authorize an
    /// app, or [`Host::unrestricted`] for trusted in-process use only.
    pub fn new() -> Self {
        Host {
            state: RuntimeState::with_broker(AppId::new(), CapabilityBroker::new()),
            limits: ResourceLimits::default(),
        }
    }

    /// A host scoped to `app`, pre-granted with `caps`. Anything not listed is
    /// denied.
    pub fn with_grants(app: impl Into<AppId>, caps: impl IntoIterator<Item = Capability>) -> Self {
        let mut state = RuntimeState::with_broker(app.into(), CapabilityBroker::new());
        for cap in caps {
            state.grant(cap);
        }
        Host {
            state,
            limits: ResourceLimits::default(),
        }
    }

    /// A host with no broker installed: all operations are permitted. This
    /// bypasses the capability model and exists only for trusted, in-process
    /// callers such as conformance tests of the legacy path.
    pub fn unrestricted() -> Self {
        Host {
            state: RuntimeState::default(),
            limits: ResourceLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Convenience accessor mirroring `RuntimeStub::element`.
    pub fn element(&self, id: dom::ElementId) -> Option<Element> {
        self.state.element(id.id)
    }

    pub fn click_count(&self, id: dom::ElementId) -> usize {
        self.state.click_count(id.id)
    }

    pub fn click_handler_ids(&self, id: dom::ElementId) -> Option<Vec<u64>> {
        self.state.click_handler_ids(id.id)
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
        dom::ElementId {
            id: self.state.create_element(tag),
        }
    }

    fn set_text(&mut self, el: dom::ElementId, text: String) {
        self.state.set_text(el.id, text);
    }

    fn set_attribute(&mut self, el: dom::ElementId, name: String, value: String) {
        self.state.set_attribute(el.id, name, value);
    }

    fn append_child(&mut self, parent: dom::ElementId, child: dom::ElementId) {
        self.state.append_child(parent.id, child.id);
    }

    fn on_click(&mut self, el: dom::ElementId, handler_id: u64) {
        self.state.on_click(el.id, handler_id);
    }
}

impl MediaHost for Host {
    fn create_player(&mut self, cfg: media::VideoConfig) -> Result<media::PlayerHandle, String> {
        self.state
            .create_player(crate::stub::VideoConfig {
                codec: cfg.codec,
                width: cfg.width,
                height: cfg.height,
                bitrate: cfg.bitrate,
            })
            .map(|id| media::PlayerHandle { handle: id })
    }

    fn play(&mut self, handle: media::PlayerHandle) {
        self.state.play(handle.handle);
    }

    fn pause(&mut self, handle: media::PlayerHandle) {
        self.state.pause(handle.handle);
    }

    fn seek(&mut self, handle: media::PlayerHandle, time_ms: u64) {
        self.state.seek(handle.handle, time_ms);
    }
}

impl NetworkHost for Host {
    fn fetch(&mut self, req: network::Request) -> Result<network::Response, String> {
        let neutral = Request {
            method: req.method,
            url: req.url,
            headers: req.headers,
            body: req.body,
        };
        self.state.fetch(neutral).map(|r| network::Response {
            status: r.status,
            headers: r.headers,
            body: r.body,
        })
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
        let limits = host.limits;
        let mut store = Store::new(engine, host);
        store.limiter(|h: &mut Host| &mut h.limits);
        store.set_fuel(limits.fuel_per_call)?;
        store.set_epoch_deadline(limits.epoch_deadline_ticks);
        let mut linker: Linker<Host> = Linker::new(engine);
        bindings::helix::runtime::dom::add_to_linker::<Host, Host>(&mut linker, |h: &mut Host| h)?;
        bindings::helix::runtime::network::add_to_linker::<Host, Host>(
            &mut linker,
            |h: &mut Host| h,
        )?;
        bindings::helix::runtime::storage::add_to_linker::<Host, Host>(
            &mut linker,
            |h: &mut Host| h,
        )?;
        bindings::helix::runtime::media::add_to_linker::<Host, Host>(
            &mut linker,
            |h: &mut Host| h,
        )?;
        let bindings = HelixGuest::instantiate(&mut store, &self.component, &linker)?;
        Ok(Instance {
            bindings,
            store,
            profiler: Profiler::new(),
        })
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
    profiler: Profiler,
}

impl Instance {
    /// Invokes the guest's exported `run` entry point under a fresh fuel budget
    /// and epoch deadline. Returns an error (a trap) if the guest exhausts its
    /// fuel, passes its epoch deadline, or hits a resource limit.
    ///
    /// Each call is timed as one `wasm.call` op on [`Instance::profiler`].
    pub fn run(&mut self) -> Result<(), wasmtime::Error> {
        let limits = self.store.data().limits;
        self.store.set_fuel(limits.fuel_per_call)?;
        self.store.set_epoch_deadline(limits.epoch_deadline_ticks);
        let _span = self.profiler.span("wasm.call");
        self.bindings.call_run(&mut self.store)?;
        Ok(())
    }

    /// Execution profile for this instance (`wasm.call` timings).
    pub fn profiler(&self) -> &Profiler {
        &self.profiler
    }

    /// Borrows the host state after (or between) guest calls.
    pub fn host(&self) -> &Host {
        self.store.data()
    }

    pub fn host_mut(&mut self) -> &mut Host {
        self.store.data_mut()
    }
}
