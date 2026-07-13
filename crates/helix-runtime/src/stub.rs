//! Runtime stub implementing the generated host-side `Guest` traits.
//!
//! `RuntimeStub` stands in for the real Helix Runtime host until the capability
//! broker is wired up (see TODO.md §"Capability broker implementation").
//!
//! The host-side contract is *generated* by `wit_bindgen` from the `helix-runtime`
//! world in `wit/helix.wit` (see `crates/helix-wit`). Because that generated
//! `Guest` trait is stateless (its methods take no `&self` — they are the
//! component's exported entry points), the stub keeps its mutable state in a
//! thread-local `RuntimeState` and `new()` resets it so conformance tests stay
//! isolated.
//!
//! `RuntimeState` is the single source of truth for the capability behavior. It
//! deliberately uses a *binding-neutral* representation (raw `u64` element ids,
//! plain request/response structs) so that both the stateless `RuntimeStub`
//! (which speaks the `wit_bindgen` `Guest` types) and the stateful wasmtime
//! `Host` (which speaks the `wasmtime`-generated `Guest` types) can delegate to
//! the same logic without the two binding crates' `ElementId`/`Request` types
//! leaking into the core.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::capability::{
    AppId, Capability, CapabilityBroker, CapabilityError, CapabilityToken, DomScope, HostPattern,
    StorageScope,
};

use helix_wit::host::exports::helix::runtime::{
    dom::{ElementId as WitElementId, Guest as DomGuest},
    media::{Guest as MediaGuest, PlayerHandle as WitPlayerHandle, VideoConfig as WitVideoConfig},
    network::{Guest as NetworkGuest, Request as WitRequest, Response as WitResponse},
    storage::Guest as StorageGuest,
};

/// Binding-neutral video configuration for the `media` interface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoConfig {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub bitrate: u32,
}

/// A media player instance tracked by the runtime.
#[derive(Debug, Clone, Default)]
pub struct Player {
    pub config: VideoConfig,
    pub playing: bool,
    pub position_ms: u64,
}

/// A node in the element tree, as seen by the stateless `RuntimeStub` API and
/// its conformance tests. `children` are `wit_bindgen` `ElementId`s to keep the
/// existing test assertions (`node.children == vec![child]`) unchanged.
#[derive(Debug, Clone, Default)]
pub struct Element {
    pub tag: String,
    pub text: String,
    pub attributes: HashMap<String, String>,
    pub children: Vec<WitElementId>,
}

/// Binding-neutral request/response, matching the `network` WIT interface but
/// independent of any codegen crate.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Internal element representation using raw `u64` ids.
#[derive(Debug, Clone, Default)]
struct InternalElement {
    tag: String,
    text: String,
    attributes: HashMap<String, String>,
    children: Vec<u64>,
}

/// Mutable host state for the capability interfaces.
///
/// Stateless-generated `Guest` impls (above) and the stateful wasmtime `Host`
/// both delegate to these methods after converting their own id/record types.
#[derive(Default)]
pub struct RuntimeState {
    next_id: u64,
    elements: HashMap<u64, InternalElement>,
    /// Click handler ids registered per element id.
    click_handlers: HashMap<u64, Vec<u64>>,
    store: HashMap<String, Vec<u8>>,
    /// Canned fetch responses keyed by exact request URL.
    fetch_responses: HashMap<String, Response>,
    /// Media players keyed by handle id.
    next_player: u64,
    players: HashMap<u64, Player>,

    /// When `Some`, capability-aware operations enforce this broker for
    /// `active_app`. When `None` the legacy permissive path is used (existing
    /// conformance tests exercise this).
    broker: Option<CapabilityBroker>,
    active_app: Option<AppId>,
    /// Resolved tokens for the active app, presented to the broker on each
    /// capability-checked operation.
    active_tokens: Vec<CapabilityToken>,
}

fn to_public(el: &InternalElement) -> Element {
    Element {
        tag: el.tag.clone(),
        text: el.text.clone(),
        attributes: el.attributes.clone(),
        children: el
            .children
            .iter()
            .map(|c| WitElementId { id: *c })
            .collect(),
    }
}

impl RuntimeState {
    // --- capability broker integration -----------------------------------

    /// Installs a broker scoped to `app`, switching this state from the legacy
    /// permissive path to capability-enforced operation.
    pub fn with_broker(app: AppId, broker: CapabilityBroker) -> Self {
        RuntimeState {
            broker: Some(broker),
            active_app: Some(app),
            ..Default::default()
        }
    }

    /// Grants `cap` to the active app and records the resulting token for
    /// subsequent capability checks. Panics if no broker is installed.
    pub fn grant(&mut self, cap: Capability) -> CapabilityToken {
        let app = self
            .active_app
            .clone()
            .expect("grant called without an active app / broker");
        let token = self
            .broker
            .as_mut()
            .expect("grant called without a broker")
            .grant(app, cap);
        self.active_tokens.push(token);
        token
    }

    /// Returns the active capability broker, if installed.
    pub fn broker(&self) -> Option<&CapabilityBroker> {
        self.broker.as_ref()
    }

    /// Returns a mutable handle to the active capability broker, so grants can
    /// be revoked (trap/abort) while a module is live.
    pub fn broker_mut(&mut self) -> Option<&mut CapabilityBroker> {
        self.broker.as_mut()
    }

    /// Enforces `cap` against the installed broker. With no broker installed
    /// (legacy path) this always succeeds. With a broker, it succeeds only if
    /// one of the active app's tokens authorizes the capability.
    pub fn check_cap(&self, cap: &Capability) -> Result<(), CapabilityError> {
        let Some(broker) = self.broker.as_ref() else {
            return Ok(());
        };
        let mut last = None;
        for token in &self.active_tokens {
            match broker.check(*token, cap) {
                Ok(_) => return Ok(()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| CapabilityError::Denied {
            app: self.active_app.clone().unwrap_or_default(),
            capability: cap.clone(),
        }))
    }

    pub fn create_element(&mut self, tag: String) -> u64 {
        if self
            .check_cap(&Capability::Dom {
                scope: DomScope::Full,
            })
            .is_err()
        {
            return u64::MAX; // denied: caller receives an unusable handle
        }
        let id = self.next_id;
        self.next_id += 1;
        self.elements.insert(
            id,
            InternalElement {
                tag,
                ..Default::default()
            },
        );
        id
    }

    pub fn set_text(&mut self, id: u64, text: String) {
        if self
            .check_cap(&Capability::Dom {
                scope: DomScope::Full,
            })
            .is_err()
        {
            return;
        }
        if let Some(node) = self.elements.get_mut(&id) {
            node.text = text;
        }
    }

    pub fn set_attribute(&mut self, id: u64, name: String, value: String) {
        if self
            .check_cap(&Capability::Dom {
                scope: DomScope::Full,
            })
            .is_err()
        {
            return;
        }
        if let Some(node) = self.elements.get_mut(&id) {
            node.attributes.insert(name, value);
        }
    }

    pub fn append_child(&mut self, parent: u64, child: u64) {
        if self
            .check_cap(&Capability::Dom {
                scope: DomScope::Full,
            })
            .is_err()
        {
            return;
        }
        if let Some(node) = self.elements.get_mut(&parent) {
            node.children.push(child);
        }
    }

    pub fn on_click(&mut self, id: u64, handler_id: u64) {
        if self
            .check_cap(&Capability::Dom {
                scope: DomScope::Full,
            })
            .is_err()
        {
            return;
        }
        self.click_handlers.entry(id).or_default().push(handler_id);
    }

    // --- media -------------------------------------------------------------

    /// Creates a media player for `cfg`. Enforces the `media` capability
    /// (resolution cap): without a broker the legacy path allows any config;
    /// with a broker the requested `(width, height)` must not exceed the
    /// granted maximum, otherwise the operation is denied (trap/abort).
    pub fn create_player(&mut self, cfg: VideoConfig) -> Result<u64, String> {
        self.check_cap(&Capability::Media {
            max_resolution: Some((cfg.width, cfg.height)),
        })
        .map_err(|e| e.to_string())?;
        let id = self.next_player;
        self.next_player += 1;
        self.players.insert(
            id,
            Player {
                config: cfg,
                ..Default::default()
            },
        );
        Ok(id)
    }

    pub fn play(&mut self, id: u64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.playing = true;
        }
    }

    pub fn pause(&mut self, id: u64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.playing = false;
        }
    }

    pub fn seek(&mut self, id: u64, time_ms: u64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.position_ms = time_ms;
        }
    }

    pub fn player(&self, id: u64) -> Option<Player> {
        self.players.get(&id).cloned()
    }

    pub fn fetch(&self, req: Request) -> Result<Response, String> {
        let host = crate::capability::host_of(&req.url);
        self.check_cap(&Capability::Network {
            hosts: vec![HostPattern::Exact(host)],
        })
        .map_err(|e| e.to_string())?;
        self.fetch_responses
            .get(&req.url)
            .cloned()
            .ok_or_else(|| format!("network capability denied or no route for {}", req.url))
    }

    pub fn get(&self, key: String) -> Option<Vec<u8>> {
        if self
            .check_cap(&Capability::Storage {
                scope: StorageScope::Namespace(key.clone()),
            })
            .is_err()
        {
            return None;
        }
        self.store.get(&key).cloned()
    }

    pub fn stored(&self, key: &str) -> Option<Vec<u8>> {
        self.store.get(key).cloned()
    }

    pub fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.check_cap(&Capability::Storage {
            scope: StorageScope::Namespace(key.clone()),
        })
        .map_err(|e| e.to_string())?;
        self.store.insert(key, value);
        Ok(())
    }

    pub fn delete(&mut self, key: String) -> Result<(), String> {
        self.check_cap(&Capability::Storage {
            scope: StorageScope::Namespace(key.clone()),
        })
        .map_err(|e| e.to_string())?;
        self.store.remove(&key);
        Ok(())
    }

    /// Registers a canned `fetch` response for an exact URL (test helper).
    pub fn register_fetch(&mut self, url: impl Into<String>, response: Response) {
        self.fetch_responses.insert(url.into(), response);
    }

    pub fn element(&self, id: u64) -> Option<Element> {
        self.elements.get(&id).map(to_public)
    }

    pub fn click_count(&self, id: u64) -> usize {
        self.click_handlers.get(&id).map(Vec::len).unwrap_or(0)
    }

    pub fn click_handler_ids(&self, id: u64) -> Option<Vec<u64>> {
        self.click_handlers.get(&id).cloned()
    }
}

thread_local! {
    static STATE: RefCell<RuntimeState> = RefCell::new(RuntimeState::default());
}

/// An in-memory stand-in for the Helix Runtime host.
///
/// `RuntimeStub` is a zero-sized marker for the host instance on the current
/// thread; all state lives in the thread-local `STATE` above. Its associated
/// functions mirror the generated `Guest` trait so conformance tests can call
/// them directly without a live component instance.
pub struct RuntimeStub;

impl RuntimeStub {
    /// Creates a fresh host instance for the calling thread (resets state).
    pub fn new() -> Self {
        STATE.with(|s| *s.borrow_mut() = RuntimeState::default());
        RuntimeStub
    }

    pub fn register_fetch(&self, url: impl Into<String>, response: WitResponse) {
        STATE.with(|s| {
            s.borrow_mut().register_fetch(
                url,
                Response {
                    status: response.status,
                    headers: response.headers,
                    body: response.body,
                },
            )
        });
    }

    pub fn element(&self, id: WitElementId) -> Option<Element> {
        STATE.with(|s| s.borrow().element(id.id))
    }

    pub fn click_count(&self, id: WitElementId) -> usize {
        STATE.with(|s| s.borrow().click_count(id.id))
    }

    pub fn click_handler_ids(&self, id: WitElementId) -> Option<Vec<u64>> {
        STATE.with(|s| s.borrow().click_handler_ids(id.id))
    }

    // --- thread-local delegations mirroring the `Guest` trait -------------

    pub fn fetch(req: WitRequest) -> Result<WitResponse, String> {
        let neutral = Request {
            method: req.method,
            url: req.url,
            headers: req.headers,
            body: req.body,
        };
        STATE
            .with(|s| s.borrow().fetch(neutral))
            .map(|r| WitResponse {
                status: r.status,
                headers: r.headers,
                body: r.body,
            })
    }

    pub fn get(key: String) -> Option<Vec<u8>> {
        STATE.with(|s| s.borrow().get(key))
    }

    pub fn set(key: String, value: Vec<u8>) -> Result<(), String> {
        STATE.with(|s| s.borrow_mut().set(key, value))
    }

    pub fn delete(key: String) -> Result<(), String> {
        STATE.with(|s| s.borrow_mut().delete(key))
    }

    pub fn create_element(tag: String) -> WitElementId {
        STATE.with(|s| WitElementId {
            id: s.borrow_mut().create_element(tag),
        })
    }

    pub fn set_text(el: WitElementId, text: String) {
        STATE.with(|s| s.borrow_mut().set_text(el.id, text))
    }

    pub fn set_attribute(el: WitElementId, name: String, value: String) {
        STATE.with(|s| s.borrow_mut().set_attribute(el.id, name, value))
    }

    pub fn append_child(parent: WitElementId, child: WitElementId) {
        STATE.with(|s| s.borrow_mut().append_child(parent.id, child.id))
    }

    pub fn on_click(el: WitElementId, handler_id: u64) {
        STATE.with(|s| s.borrow_mut().on_click(el.id, handler_id))
    }

    // --- media (stateless delegations mirroring the `Guest` trait) --------

    pub fn create_player(cfg: WitVideoConfig) -> Result<WitPlayerHandle, String> {
        let neutral = VideoConfig {
            codec: cfg.codec,
            width: cfg.width,
            height: cfg.height,
            bitrate: cfg.bitrate,
        };
        STATE
            .with(|s| s.borrow_mut().create_player(neutral))
            .map(|id| WitPlayerHandle { handle: id })
    }

    pub fn play(handle: WitPlayerHandle) {
        STATE.with(|s| s.borrow_mut().play(handle.handle));
    }

    pub fn pause(handle: WitPlayerHandle) {
        STATE.with(|s| s.borrow_mut().pause(handle.handle));
    }

    pub fn seek(handle: WitPlayerHandle, time_ms: u64) {
        STATE.with(|s| s.borrow_mut().seek(handle.handle, time_ms));
    }

    pub fn player(handle: WitPlayerHandle) -> Option<Player> {
        STATE.with(|s| s.borrow().player(handle.handle))
    }
}

impl Default for RuntimeStub {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkGuest for RuntimeStub {
    fn fetch(req: WitRequest) -> Result<WitResponse, String> {
        Self::fetch(req)
    }
}

impl StorageGuest for RuntimeStub {
    fn get(key: String) -> Option<Vec<u8>> {
        Self::get(key)
    }

    fn set(key: String, value: Vec<u8>) -> Result<(), String> {
        Self::set(key, value)
    }

    fn delete(key: String) -> Result<(), String> {
        Self::delete(key)
    }
}

impl DomGuest for RuntimeStub {
    fn create_element(tag: String) -> WitElementId {
        Self::create_element(tag)
    }

    fn set_text(el: WitElementId, text: String) {
        Self::set_text(el, text)
    }

    fn set_attribute(el: WitElementId, name: String, value: String) {
        Self::set_attribute(el, name, value)
    }

    fn append_child(parent: WitElementId, child: WitElementId) {
        Self::append_child(parent, child)
    }

    fn on_click(el: WitElementId, handler_id: u64) {
        Self::on_click(el, handler_id)
    }
}

impl MediaGuest for RuntimeStub {
    fn create_player(cfg: WitVideoConfig) -> Result<WitPlayerHandle, String> {
        Self::create_player(cfg)
    }

    fn play(handle: WitPlayerHandle) {
        Self::play(handle)
    }

    fn pause(handle: WitPlayerHandle) {
        Self::pause(handle)
    }

    fn seek(handle: WitPlayerHandle, time_ms: u64) {
        Self::seek(handle, time_ms)
    }
}
