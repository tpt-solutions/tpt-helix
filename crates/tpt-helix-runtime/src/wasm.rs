//! Guest execution on tpt-wasm.
//!
//! A guest is a componentized module built from the `helix-guest` world (see
//! `crates/tpt-helix-wit/wit/helix.wit`). Its imports are the `dom`, `storage`,
//! `network` and `media` interfaces. Each import is supplied to the component layer as a typed
//! host function that forwards to the binding-neutral [`RuntimeState`], which enforces the
//! capability broker. The guest's `run` and `dispatch` exports are called through the component
//! layer.
//!
//! The per-interface host traits live in [`bindings`]. They mirror the names of the traits the
//! WIT world generates, so callers can use the host state directly.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use tpt_wasm_capability::{
    CapabilityDescriptor, CapabilityGrant, CapabilityId, CapabilitySet, Operation, Permissions,
};
use tpt_wasm_component::wit::WitDocument;
use tpt_wasm_component::{Component, ComponentInstance, ComponentValue, Imports};
use tpt_wasm_runtime::{Config, Engine as RuntimeEngine};
use tpt_wasm_types::ResourceLimits as EngineLimits;

use crate::capability::{AppId, Capability, CapabilityBroker};
use crate::profiler::Profiler;
use crate::stub::{Element, Request, RuntimeState};

/// The `helix-guest` world, the source of every import's signature.
const WORLD: &str = include_str!("../../tpt-helix-wit/wit/helix.wit");

/// Every interface function the world imports, by interface and name.
const IMPORTS: [(&str, &str); 14] = [
    ("dom", "create-element"),
    ("dom", "set-text"),
    ("dom", "set-attribute"),
    ("dom", "append-child"),
    ("dom", "on-click"),
    ("dom", "on-event"),
    ("network", "fetch"),
    ("storage", "get"),
    ("storage", "set"),
    ("storage", "delete"),
    ("media", "create-player"),
    ("media", "play"),
    ("media", "pause"),
    ("media", "seek"),
];

/// An error from loading, instantiating, or running a guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<tpt_wasm_component::ComponentError> for Error {
    fn from(error: tpt_wasm_component::ComponentError) -> Self {
        Error(error.to_string())
    }
}

impl From<tpt_wasm_runtime::RuntimeError> for Error {
    fn from(error: tpt_wasm_runtime::RuntimeError) -> Self {
        Error(error.to_string())
    }
}

/// The engine a guest is loaded and instantiated on. Only an engine from [`sandboxed_engine`]
/// instantiates a guest: the per-call budget is applied when an instance is created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Engine {
    sandboxed: bool,
}

/// Builds an [`Engine`] with the sandbox the runtime relies on: a per-call execution budget and
/// the memory and table limits in [`ResourceLimits`]. An engine from [`Engine::default`] has
/// neither, so instantiating a guest on it fails.
pub fn sandboxed_engine() -> Result<Engine, Error> {
    RuntimeEngine::new(Config::default())?;
    Ok(Engine { sandboxed: true })
}

/// Resource budget applied to every guest instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum linear memory a guest may grow to, in bytes.
    pub max_memory_bytes: usize,
    /// Maximum number of elements a guest table may grow to.
    pub max_table_elements: usize,
    /// Execution steps granted to each call (`run` or `dispatch`). Exhaustion traps the guest.
    pub fuel_per_call: u64,
    /// Epoch ticks until the wall-clock deadline fires. Not enforced yet: tpt-wasm has no
    /// wall-clock deadline, so the per-call step budget is the only time bound.
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

impl ResourceLimits {
    /// Whether a memory may grow from `current` to `desired` bytes.
    pub fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, Error> {
        Ok(desired <= self.max_memory_bytes)
    }

    /// Whether a table may grow from `current` to `desired` elements.
    pub fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, Error> {
        Ok(desired <= self.max_table_elements)
    }

    /// The engine configuration these limits imply: the memory cap in 64 KiB pages, the table cap,
    /// and the step budget per call.
    fn engine_config(&self) -> Config {
        let mut config = Config::default();
        config.limits = EngineLimits {
            max_memory_pages: (self.max_memory_bytes as u64).div_ceil(65536),
            max_table_elements: u32::try_from(self.max_table_elements).unwrap_or(u32::MAX),
            max_execution_steps: Some(self.fuel_per_call),
            ..EngineLimits::default()
        };
        config
    }
}

/// The host-side traits of the `helix-guest` world's interfaces. Each mirrors the trait the WIT
/// world generates for its interface: the same names, and the same argument and result types.
pub mod bindings {
    pub mod helix {
        pub mod runtime {
            pub mod dom {
                #[derive(Debug, Clone, Copy, PartialEq, Eq)]
                pub struct ElementId {
                    pub id: u64,
                }

                pub trait Host {
                    fn create_element(&mut self, tag: String) -> ElementId;
                    fn set_text(&mut self, el: ElementId, text: String);
                    fn set_attribute(&mut self, el: ElementId, name: String, value: String);
                    fn append_child(&mut self, parent: ElementId, child: ElementId);
                    fn on_click(&mut self, el: ElementId, handler_id: u64);
                    fn on_event(&mut self, el: ElementId, event: String, handler_id: u64);
                }
            }

            pub mod storage {
                pub trait Host {
                    fn get(&mut self, key: String) -> Option<Vec<u8>>;
                    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String>;
                    fn delete(&mut self, key: String) -> Result<(), String>;
                }
            }

            pub mod network {
                #[derive(Debug, Clone, PartialEq, Eq)]
                pub struct Request {
                    pub method: String,
                    pub url: String,
                    pub headers: Vec<(String, String)>,
                    pub body: Option<Vec<u8>>,
                }

                #[derive(Debug, Clone, PartialEq, Eq)]
                pub struct Response {
                    pub status: u16,
                    pub headers: Vec<(String, String)>,
                    pub body: Vec<u8>,
                }

                pub trait Host {
                    fn fetch(&mut self, req: Request) -> Result<Response, String>;
                }
            }

            pub mod media {
                #[derive(Debug, Clone, PartialEq, Eq)]
                pub struct VideoConfig {
                    pub codec: String,
                    pub width: u32,
                    pub height: u32,
                    pub bitrate: u32,
                }

                #[derive(Debug, Clone, Copy, PartialEq, Eq)]
                pub struct PlayerHandle {
                    pub handle: u64,
                }

                pub trait Host {
                    fn create_player(&mut self, cfg: VideoConfig) -> Result<PlayerHandle, String>;
                    fn play(&mut self, handle: PlayerHandle);
                    fn pause(&mut self, handle: PlayerHandle);
                    fn seek(&mut self, handle: PlayerHandle, time_ms: u64);
                }
            }
        }
    }
}

use bindings::helix::runtime::{dom, media, network};
use bindings::helix::runtime::{
    dom::Host as DomHost, media::Host as MediaHost, network::Host as NetworkHost,
    storage::Host as StorageHost,
};

/// Host state handed to a guest component. Implements the host traits by delegating to a
/// binding-neutral [`RuntimeState`].
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

    fn on_event(&mut self, el: dom::ElementId, event: String, handler_id: u64) {
        self.state.on_event(el.id, event, handler_id);
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

/// A guest component, decoded and ready to instantiate.
pub struct Module {
    component: Component,
    source_len: usize,
}

impl Module {
    /// Decodes and validates `bytes` as a WASM component. The engine is checked when the module
    /// is instantiated.
    pub fn load(_engine: &Engine, bytes: &[u8]) -> Result<Self, Error> {
        Ok(Module {
            component: Component::decode(bytes)?,
            source_len: bytes.len(),
        })
    }

    /// Number of bytes in the source the module was loaded from.
    pub fn source_len(&self) -> usize {
        self.source_len
    }

    /// Instantiates the module against `host`, supplying the world's imports as typed host
    /// functions over the host state. Consumes `host`, which is reachable through
    /// [`Instance::host`] and [`Instance::host_mut`].
    pub fn instantiate(&self, engine: &Engine, host: Host) -> Result<Instance, Error> {
        if !engine.sandboxed {
            return Err(Error(
                "the engine has no sandbox: build it with sandboxed_engine".to_owned(),
            ));
        }
        let limits = host.limits;
        let runtime = RuntimeEngine::new(limits.engine_config())?;
        let host = Arc::new(Mutex::new(host));
        let imports = helix_imports(&host)?;
        let component = self.component.instantiate_with(&runtime, &imports)?;
        Ok(Instance {
            component,
            host,
            profiler: Profiler::new(),
            _runtime: runtime,
        })
    }
}

/// The world's imports, each a typed host function that forwards to the host state. Every import
/// is granted `call`: the capability broker inside the host state decides each operation.
fn helix_imports(host: &Arc<Mutex<Host>>) -> Result<Imports, Error> {
    let document = WitDocument::parse(WORLD).map_err(|error| Error(error.to_string()))?;
    let mut capabilities = CapabilitySet::default();
    let mut typed = Vec::with_capacity(IMPORTS.len());
    for (interface, function) in IMPORTS {
        let (table, ty) = document
            .function_signature(interface, function)
            .map_err(|error| Error(error.to_string()))?;
        let name = format!("helix:runtime/{interface}@0.1.0#{function}");
        capabilities
            .register(CapabilityDescriptor {
                id: CapabilityId::new(&name),
                operations: vec![Operation::execute("call")],
            })
            .map_err(|error| Error(format!("{error:?}")))?;
        capabilities
            .grant(CapabilityGrant {
                capability: CapabilityId::new(&name),
                permissions: Permissions {
                    execute: true,
                    ..Permissions::default()
                },
            })
            .map_err(|error| Error(format!("{error:?}")))?;
        typed.push((name, table, ty, format!("{interface}.{function}")));
    }
    let mut imports = Imports::with_capabilities(Arc::new(Mutex::new(capabilities)));
    for (name, table, ty, operation) in typed {
        let host = host.clone();
        imports.define_typed_function(name, table, ty, move |args: &[ComponentValue]| {
            call_host(&host, &operation, args)
        });
    }
    Ok(imports)
}

/// Run one import against the host state. `operation` is `interface.function`.
fn call_host(
    host: &Mutex<Host>,
    operation: &str,
    args: &[ComponentValue],
) -> Result<Vec<ComponentValue>, String> {
    let mut host = host
        .lock()
        .map_err(|_| "the host state is unavailable".to_owned())?;
    let host = &mut *host;
    Ok(match operation {
        "dom.create-element" => {
            let tag = text(args, 0)?;
            vec![element_value(DomHost::create_element(host, tag))]
        }
        "dom.set-text" => {
            DomHost::set_text(host, element(args, 0)?, text(args, 1)?);
            Vec::new()
        }
        "dom.set-attribute" => {
            DomHost::set_attribute(host, element(args, 0)?, text(args, 1)?, text(args, 2)?);
            Vec::new()
        }
        "dom.append-child" => {
            DomHost::append_child(host, element(args, 0)?, element(args, 1)?);
            Vec::new()
        }
        "dom.on-click" => {
            DomHost::on_click(host, element(args, 0)?, u64_arg(args, 1)?);
            Vec::new()
        }
        "dom.on-event" => {
            DomHost::on_event(
                host,
                element(args, 0)?,
                text(args, 1)?,
                u64_arg(args, 2)?,
            );
            Vec::new()
        }
        "network.fetch" => {
            let request = request(args, 0)?;
            vec![match NetworkHost::fetch(host, request) {
                Ok(response) => result_ok(response_value(response)),
                Err(message) => result_err(message),
            }]
        }
        "storage.get" => {
            let found = StorageHost::get(host, text(args, 0)?);
            vec![ComponentValue::Option(
                found.map(|bytes| Box::new(bytes_value(&bytes))),
            )]
        }
        "storage.set" => {
            let outcome = StorageHost::set(host, text(args, 0)?, bytes(args, 1)?);
            vec![unit_result(outcome)]
        }
        "storage.delete" => {
            let outcome = StorageHost::delete(host, text(args, 0)?);
            vec![unit_result(outcome)]
        }
        "media.create-player" => {
            let config = video_config(args, 0)?;
            vec![match MediaHost::create_player(host, config) {
                Ok(handle) => result_ok(player_value(handle)),
                Err(message) => result_err(message),
            }]
        }
        "media.play" => {
            MediaHost::play(host, player(args, 0)?);
            Vec::new()
        }
        "media.pause" => {
            MediaHost::pause(host, player(args, 0)?);
            Vec::new()
        }
        "media.seek" => {
            MediaHost::seek(host, player(args, 0)?, u64_arg(args, 1)?);
            Vec::new()
        }
        other => return Err(format!("the host has no function {other}")),
    })
}

fn element_value(el: dom::ElementId) -> ComponentValue {
    ComponentValue::Record(vec![("id".to_owned(), ComponentValue::U64(el.id))])
}

fn element(args: &[ComponentValue], at: usize) -> Result<dom::ElementId, String> {
    Ok(dom::ElementId {
        id: field_u64(arg(args, at)?, "id")?,
    })
}

fn player(args: &[ComponentValue], at: usize) -> Result<media::PlayerHandle, String> {
    Ok(media::PlayerHandle {
        handle: field_u64(arg(args, at)?, "handle")?,
    })
}

fn video_config(args: &[ComponentValue], at: usize) -> Result<media::VideoConfig, String> {
    let record = arg(args, at)?;
    Ok(media::VideoConfig {
        codec: text_of(field(record, "codec")?)?,
        width: u32_of(field(record, "width")?)?,
        height: u32_of(field(record, "height")?)?,
        bitrate: u32_of(field(record, "bitrate")?)?,
    })
}

fn request(args: &[ComponentValue], at: usize) -> Result<network::Request, String> {
    let record = arg(args, at)?;
    let headers = match field(record, "headers")? {
        ComponentValue::List(items) => items.iter().map(header).collect::<Result<_, _>>()?,
        other => return Err(format!("headers is {other:?}, not a list")),
    };
    let body = match field(record, "body")? {
        ComponentValue::Option(None) => None,
        ComponentValue::Option(Some(value)) => Some(bytes_of(value)?),
        other => return Err(format!("body is {other:?}, not an option")),
    };
    Ok(network::Request {
        method: text_of(field(record, "method")?)?,
        url: text_of(field(record, "url")?)?,
        headers,
        body,
    })
}

fn response_value(response: network::Response) -> ComponentValue {
    ComponentValue::Record(vec![
        ("status".to_owned(), ComponentValue::U16(response.status)),
        (
            "headers".to_owned(),
            ComponentValue::List(
                response
                    .headers
                    .into_iter()
                    .map(|(name, value)| {
                        ComponentValue::Tuple(vec![
                            ComponentValue::String(name),
                            ComponentValue::String(value),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("body".to_owned(), bytes_value(&response.body)),
    ])
}

/// A `result<T, string>` holding its ok payload.
fn result_ok(value: ComponentValue) -> ComponentValue {
    ComponentValue::Result(Ok(Some(Box::new(value))))
}

/// A `result<T, string>` holding its error message.
fn result_err(message: String) -> ComponentValue {
    ComponentValue::Result(Err(Some(Box::new(ComponentValue::String(message)))))
}

/// A `result<_, string>` value.
fn unit_result(outcome: Result<(), String>) -> ComponentValue {
    match outcome {
        Ok(()) => ComponentValue::Result(Ok(None)),
        Err(message) => ComponentValue::Result(Err(Some(Box::new(ComponentValue::String(message))))),
    }
}

fn player_value(handle: media::PlayerHandle) -> ComponentValue {
    ComponentValue::Record(vec![(
        "handle".to_owned(),
        ComponentValue::U64(handle.handle),
    )])
}

fn bytes_value(bytes: &[u8]) -> ComponentValue {
    ComponentValue::List(bytes.iter().map(|byte| ComponentValue::U8(*byte)).collect())
}

fn header(value: &ComponentValue) -> Result<(String, String), String> {
    match value {
        ComponentValue::Tuple(pair) => match pair.as_slice() {
            [name, value] => Ok((text_of(name)?, text_of(value)?)),
            _ => Err("a header is not a pair".to_owned()),
        },
        other => Err(format!("a header is {other:?}, not a tuple")),
    }
}

fn arg(args: &[ComponentValue], at: usize) -> Result<&ComponentValue, String> {
    args.get(at)
        .ok_or_else(|| format!("argument {at} is missing"))
}

fn field<'a>(record: &'a ComponentValue, name: &str) -> Result<&'a ComponentValue, String> {
    match record {
        ComponentValue::Record(fields) => fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
            .ok_or_else(|| format!("the record has no field {name}")),
        other => Err(format!("{other:?} is not a record")),
    }
}

fn field_u64(record: &ComponentValue, name: &str) -> Result<u64, String> {
    match field(record, name)? {
        ComponentValue::U64(value) => Ok(*value),
        other => Err(format!("{name} is {other:?}, not a u64")),
    }
}

fn text(args: &[ComponentValue], at: usize) -> Result<String, String> {
    text_of(arg(args, at)?)
}

fn text_of(value: &ComponentValue) -> Result<String, String> {
    match value {
        ComponentValue::String(text) => Ok(text.clone()),
        other => Err(format!("{other:?} is not a string")),
    }
}

fn u64_arg(args: &[ComponentValue], at: usize) -> Result<u64, String> {
    match arg(args, at)? {
        ComponentValue::U64(value) => Ok(*value),
        other => Err(format!("{other:?} is not a u64")),
    }
}

fn u32_of(value: &ComponentValue) -> Result<u32, String> {
    match value {
        ComponentValue::U32(value) => Ok(*value),
        other => Err(format!("{other:?} is not a u32")),
    }
}

fn bytes(args: &[ComponentValue], at: usize) -> Result<Vec<u8>, String> {
    bytes_of(arg(args, at)?)
}

fn bytes_of(value: &ComponentValue) -> Result<Vec<u8>, String> {
    match value {
        ComponentValue::List(items) => items
            .iter()
            .map(|item| match item {
                ComponentValue::U8(byte) => Ok(*byte),
                other => Err(format!("{other:?} is not a byte")),
            })
            .collect(),
        other => Err(format!("{other:?} is not a list of bytes")),
    }
}

/// A live guest instance.
///
/// Teardown is implicit: dropping the `Instance` releases the guest and its store. Call
/// [`Instance::run`] to drive the module's exported `run` entry point.
pub struct Instance {
    component: ComponentInstance,
    host: Arc<Mutex<Host>>,
    profiler: Profiler,
    /// The engine the instance runs on, kept alive for as long as the instance is.
    _runtime: RuntimeEngine,
}

impl Instance {
    /// Invokes the guest's exported `run` entry point. Each call gets a fresh step budget. A
    /// trap, such as exhausting the budget, is returned as an error.
    ///
    /// Each call is timed as one `wasm.call` op on [`Instance::profiler`].
    pub fn run(&mut self) -> Result<(), Error> {
        let _span = self.profiler.span("wasm.call");
        self.component.call_export("run", Vec::new())?;
        Ok(())
    }

    /// Calls the guest's exported `dispatch` for one handler id, under a fresh step budget. A trap
    /// is returned to the caller.
    pub fn dispatch_handler(&mut self, handler_id: u64) -> Result<(), Error> {
        let _span = self.profiler.span("wasm.dispatch");
        self.component
            .call_export("dispatch", vec![ComponentValue::U64(handler_id)])?;
        Ok(())
    }

    /// Fires DOM event `event` on element `el`: each handler registered for it
    /// is dispatched to the guest, in registration order. Returns how many ran.
    ///
    /// Dispatch stops at the first trap, and that trap is returned. Handlers
    /// that already ran keep their effects. Events with no handlers do nothing.
    pub fn fire_event(&mut self, el: u64, event: &str) -> Result<usize, Error> {
        let handlers = self.host().state.handler_ids(el, event);
        for handler_id in &handlers {
            self.dispatch_handler(*handler_id)?;
        }
        Ok(handlers.len())
    }

    /// Execution profile for this instance (`wasm.call` timings).
    pub fn profiler(&self) -> &Profiler {
        &self.profiler
    }

    /// Borrows the host state after (or between) guest calls.
    pub fn host(&self) -> MutexGuard<'_, Host> {
        self.host.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn host_mut(&mut self) -> MutexGuard<'_, Host> {
        self.host()
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::capability::DomScope;

    #[test]
    fn limiter_denies_growth_past_the_budget() {
        let mut limits = ResourceLimits {
            max_memory_bytes: 1024,
            max_table_elements: 8,
            ..Default::default()
        };
        assert!(limits.memory_growing(0, 1024, None).unwrap());
        assert!(!limits.memory_growing(1024, 1025, None).unwrap());
        assert!(limits.table_growing(0, 8, None).unwrap());
        assert!(!limits.table_growing(8, 9, None).unwrap());
    }

    #[test]
    fn default_host_denies_element_creation_until_granted() {
        let mut host = Host::new();
        let el = DomHost::create_element(&mut host, "div".to_string());
        assert_eq!(
            el.id,
            u64::MAX,
            "ungranted guest must get the unusable handle"
        );
    }

    #[test]
    fn unrestricted_host_allows_element_creation() {
        let mut host = Host::unrestricted();
        let el = DomHost::create_element(&mut host, "div".to_string());
        assert_ne!(el.id, u64::MAX);
    }

    #[test]
    fn full_dom_grant_allows_creation_and_subtree_grant_does_not() {
        let mut full = Host::with_grants(
            "wasm-test-full",
            [Capability::Dom {
                scope: DomScope::Full,
            }],
        );
        let el = DomHost::create_element(&mut full, "div".to_string());
        assert_ne!(el.id, u64::MAX);

        // A subtree grant cannot create elements; only Full scope may.
        let mut subtree = Host::with_grants(
            "wasm-test-subtree",
            [Capability::Dom {
                scope: DomScope::Subtree(0),
            }],
        );
        let denied = DomHost::create_element(&mut subtree, "div".to_string());
        assert_eq!(denied.id, u64::MAX);
    }

    #[test]
    fn module_load_rejects_non_wasm_bytes() {
        let engine = sandboxed_engine().expect("sandboxed engine builds");
        assert!(Module::load(&engine, b"definitely not a wasm component").is_err());
    }

    #[test]
    fn default_limits_are_bounded() {
        let d = ResourceLimits::default();
        assert!(d.max_memory_bytes > 0 && d.max_memory_bytes <= 256 * 1024 * 1024);
        assert!(d.fuel_per_call > 0);
        assert!(d.epoch_deadline_ticks > 0);
    }
}
