//! Runtime stub implementing the generated host-side `Guest` traits.
//!
//! `RuntimeStub` is the host-side implementation of the `helix.wit` interfaces
//! backed by thread-local state. Capability enforcement goes through the
//! broker when one is installed (see `RuntimeState::with_broker`).
//!
//! The host-side contract is *generated* by `wit_bindgen` from the `helix-runtime`
//! world in `wit/helix.wit` (see `crates/tpt-helix-wit`). Because that generated
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

use tpt_helix_wit::host::exports::helix::runtime::{
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

/// Per-app guest state caps (hostile-input hardening, TODO review follow-ups).
/// A guest that exceeds a cap is denied rather than allowed to grow host memory
/// without bound: `create_element` returns the unusable handle `u64::MAX`, and
/// `set`/`on_click` drop the operation.
pub const MAX_ELEMENTS: usize = 100_000;
/// Total click handlers across all elements.
pub const MAX_CLICK_HANDLERS: usize = 100_000;
/// Distinct storage keys per app.
pub const MAX_STORAGE_ENTRIES: usize = 10_000;
/// Longest accepted storage key, in bytes.
pub const MAX_STORAGE_KEY_BYTES: usize = 256;
/// Largest accepted storage value, in bytes.
pub const MAX_STORAGE_VALUE_BYTES: usize = 1024 * 1024;

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
    /// Reverse of `InternalElement::children`: element id -> parent ids. Used to
    /// decide whether an element lies inside a granted `DomScope::Subtree`.
    parents: HashMap<u64, Vec<u64>>,
    /// Click handler ids registered per element id.
    click_handlers: HashMap<u64, Vec<u64>>,
    /// Total entries across `click_handlers`, kept so the cap check is O(1).
    click_handler_total: usize,
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
        let err = last.unwrap_or_else(|| CapabilityError::Denied {
            app: self.active_app.clone().unwrap_or_default(),
            capability: cap.clone(),
        });
        self.audit_denied(cap, &err);
        Err(err)
    }

    /// Capability-denial audit log (target `helix::capability`). Emits the
    /// denied capability, the app, and an actionable remediation hint; visible
    /// via `RUST_LOG=helix::capability=warn` once [`crate::init_logging`] runs.
    fn audit_denied(&self, cap: &Capability, err: &CapabilityError) {
        tracing::warn!(
            target: "helix::capability",
            app = %self.active_app.clone().unwrap_or_default(),
            capability = ?cap,
            reason = %err,
            hint = crate::capability::remediation_hint(cap),
            "capability denied"
        );
    }

    /// Enforces DOM authorization for an operation on element `id`.
    ///
    /// Creating elements needs `DomScope::Full` (see [`Self::create_element`]).
    /// Modifying an existing element needs a grant whose `Subtree` root is the
    /// element itself or one of its ancestors; a `Full` grant covers every
    /// subtree. Subtree grants therefore cannot reach outside their root.
    fn check_dom_target(&self, id: u64) -> Result<(), CapabilityError> {
        let Some(broker) = self.broker.as_ref() else {
            return Ok(());
        };
        let roots = self.ancestors_or_self(id);
        for token in &self.active_tokens {
            for root in &roots {
                if broker
                    .check(
                        *token,
                        &Capability::Dom {
                            scope: DomScope::Subtree(*root),
                        },
                    )
                    .is_ok()
                {
                    return Ok(());
                }
            }
        }
        let cap = Capability::Dom {
            scope: DomScope::Subtree(id),
        };
        let err = CapabilityError::Denied {
            app: self.active_app.clone().unwrap_or_default(),
            capability: cap.clone(),
        };
        self.audit_denied(&cap, &err);
        Err(err)
    }

    /// `id` followed by every element that (transitively) contains it.
    fn ancestors_or_self(&self, id: u64) -> Vec<u64> {
        let mut out = vec![id];
        let mut seen = std::collections::HashSet::from([id]);
        let mut i = 0;
        while i < out.len() {
            if let Some(parents) = self.parents.get(&out[i]) {
                for p in parents {
                    if seen.insert(*p) {
                        out.push(*p);
                    }
                }
            }
            i += 1;
        }
        out
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
        if self.elements.len() >= MAX_ELEMENTS {
            return u64::MAX; // element cap reached
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
        if self.check_dom_target(id).is_err() {
            return;
        }
        if let Some(node) = self.elements.get_mut(&id) {
            node.text = text;
        }
    }

    pub fn set_attribute(&mut self, id: u64, name: String, value: String) {
        if self.check_dom_target(id).is_err() {
            return;
        }
        if let Some(node) = self.elements.get_mut(&id) {
            node.attributes.insert(name, value);
        }
    }

    pub fn append_child(&mut self, parent: u64, child: u64) {
        // Both ends must be inside the caller's granted subtree.
        if self.check_dom_target(parent).is_err() || self.check_dom_target(child).is_err() {
            return;
        }
        if let Some(node) = self.elements.get_mut(&parent) {
            node.children.push(child);
            let parents = self.parents.entry(child).or_default();
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
    }

    pub fn on_click(&mut self, id: u64, handler_id: u64) {
        if self.check_dom_target(id).is_err() {
            return;
        }
        if self.click_handler_total >= MAX_CLICK_HANDLERS {
            return;
        }
        self.click_handler_total += 1;
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
        if host.is_empty() {
            return Err(format!("network: no host in {:?}", req.url));
        }
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
        if key.len() > MAX_STORAGE_KEY_BYTES {
            return Err(format!(
                "storage: key exceeds {MAX_STORAGE_KEY_BYTES} bytes"
            ));
        }
        if value.len() > MAX_STORAGE_VALUE_BYTES {
            return Err(format!(
                "storage: value exceeds {MAX_STORAGE_VALUE_BYTES} bytes"
            ));
        }
        if !self.store.contains_key(&key) && self.store.len() >= MAX_STORAGE_ENTRIES {
            return Err(format!("storage: more than {MAX_STORAGE_ENTRIES} keys"));
        }
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

#[cfg(test)]
mod guest_state_cap_tests {
    use super::*;

    #[test]
    fn element_creation_stops_at_the_cap() {
        let mut st = RuntimeState::default();
        for _ in 0..MAX_ELEMENTS {
            assert_ne!(st.create_element("div".into()), u64::MAX);
        }
        // Over the cap: the guest gets the unusable handle, and nothing is stored.
        assert_eq!(st.create_element("div".into()), u64::MAX);
        assert_eq!(st.elements.len(), MAX_ELEMENTS);
    }

    #[test]
    fn click_handlers_stop_at_the_cap() {
        let mut st = RuntimeState::default();
        let el = st.create_element("button".into());
        for h in 0..(MAX_CLICK_HANDLERS as u64 + 5) {
            st.on_click(el, h);
        }
        assert_eq!(st.click_count(el), MAX_CLICK_HANDLERS);
    }

    #[test]
    fn storage_key_and_value_sizes_are_capped() {
        let mut st = RuntimeState::default();
        let long_key = "k".repeat(MAX_STORAGE_KEY_BYTES + 1);
        assert!(st.set(long_key, vec![1]).is_err());
        let big = vec![0u8; MAX_STORAGE_VALUE_BYTES + 1];
        assert!(st.set("ok".into(), big).is_err());
        // Exactly at the limits is accepted.
        let edge_key = "k".repeat(MAX_STORAGE_KEY_BYTES);
        let edge_val = vec![0u8; MAX_STORAGE_VALUE_BYTES];
        assert!(st.set(edge_key, edge_val).is_ok());
    }

    #[test]
    fn storage_key_count_is_capped_but_overwrites_still_work() {
        let mut st = RuntimeState::default();
        for i in 0..MAX_STORAGE_ENTRIES {
            st.set(format!("key-{i}"), vec![0]).unwrap();
        }
        assert!(st.set("one-too-many".into(), vec![0]).is_err());
        // Overwriting an existing key does not grow the store, so it stays allowed.
        assert!(st.set("key-0".into(), vec![9]).is_ok());
        assert_eq!(st.stored("key-0"), Some(vec![9]));
    }
}
