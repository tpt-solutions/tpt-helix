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
//! `RuntimeState` is the single source of truth for the capability behavior; the
//! wasmtime host (`crate::wasm::Host`) holds its own `RuntimeState` by value and
//! delegates to the same methods, so the stub and the real engine cannot drift.

use std::cell::RefCell;
use std::collections::HashMap;

use helix_wit::host::exports::helix::runtime::{
    dom::{ElementId, Guest as DomGuest},
    network::{Guest as NetworkGuest, Request, Response},
    storage::Guest as StorageGuest,
};

/// A node in the stub's element tree.
#[derive(Debug, Clone, Default)]
pub struct Element {
    pub tag: String,
    pub text: String,
    pub attributes: HashMap<String, String>,
    pub children: Vec<ElementId>,
}

/// Mutable host state for the capability interfaces.
///
/// Stateless-generated `Guest` impls (above) and the stateful wasmtime `Host`
/// both delegate to these methods.
#[derive(Default)]
pub struct RuntimeState {
    next_id: u64,
    elements: HashMap<u64, Element>,
    /// Click handler ids registered per element id.
    click_handlers: HashMap<u64, Vec<u64>>,
    store: HashMap<String, Vec<u8>>,
    /// Canned fetch responses keyed by exact request URL.
    fetch_responses: HashMap<String, Response>,
}

impl RuntimeState {
    pub fn create_element(&mut self, tag: String) -> ElementId {
        let id = self.next_id;
        self.next_id += 1;
        self.elements.insert(
            id,
            Element {
                tag,
                ..Default::default()
            },
        );
        ElementId { id }
    }

    pub fn set_text(&mut self, el: ElementId, text: String) {
        if let Some(node) = self.elements.get_mut(&el.id) {
            node.text = text;
        }
    }

    pub fn set_attribute(&mut self, el: ElementId, name: String, value: String) {
        if let Some(node) = self.elements.get_mut(&el.id) {
            node.attributes.insert(name, value);
        }
    }

    pub fn append_child(&mut self, parent: ElementId, child: ElementId) {
        if let Some(node) = self.elements.get_mut(&parent.id) {
            node.children.push(child);
        }
    }

    pub fn on_click(&mut self, el: ElementId, handler_id: u64) {
        self.click_handlers
            .entry(el.id)
            .or_default()
            .push(handler_id);
    }

    pub fn fetch(&self, req: Request) -> Result<Response, String> {
        self.fetch_responses
            .get(&req.url)
            .cloned()
            .ok_or_else(|| format!("network capability denied or no route for {}", req.url))
    }

    pub fn get(&self, key: String) -> Option<Vec<u8>> {
        self.store.get(&key).cloned()
    }

    pub fn stored(&self, key: &str) -> Option<Vec<u8>> {
        self.store.get(key).cloned()
    }

    pub fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.store.insert(key, value);
        Ok(())
    }

    pub fn delete(&mut self, key: String) -> Result<(), String> {
        self.store.remove(&key);
        Ok(())
    }

    /// Registers a canned `fetch` response for an exact URL (test helper).
    pub fn register_fetch(&mut self, url: impl Into<String>, response: Response) {
        self.fetch_responses.insert(url.into(), response);
    }

    pub fn element(&self, id: ElementId) -> Option<Element> {
        self.elements.get(&id.id).cloned()
    }

    pub fn click_count(&self, id: ElementId) -> usize {
        self.click_handlers.get(&id.id).map(Vec::len).unwrap_or(0)
    }

    pub fn click_handler_ids(&self, id: ElementId) -> Option<Vec<u64>> {
        self.click_handlers.get(&id.id).cloned()
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

    pub fn register_fetch(&self, url: impl Into<String>, response: Response) {
        STATE.with(|s| s.borrow_mut().register_fetch(url, response));
    }

    pub fn element(&self, id: ElementId) -> Option<Element> {
        STATE.with(|s| s.borrow().element(id))
    }

    pub fn click_count(&self, id: ElementId) -> usize {
        STATE.with(|s| s.borrow().click_count(id))
    }

    pub fn click_handler_ids(&self, id: ElementId) -> Option<Vec<u64>> {
        STATE.with(|s| s.borrow().click_handler_ids(id))
    }

    // --- thread-local delegations mirroring the `Guest` trait -------------

    pub fn fetch(req: Request) -> Result<Response, String> {
        STATE.with(|s| s.borrow().fetch(req))
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

    pub fn create_element(tag: String) -> ElementId {
        STATE.with(|s| s.borrow_mut().create_element(tag))
    }

    pub fn set_text(el: ElementId, text: String) {
        STATE.with(|s| s.borrow_mut().set_text(el, text))
    }

    pub fn set_attribute(el: ElementId, name: String, value: String) {
        STATE.with(|s| s.borrow_mut().set_attribute(el, name, value))
    }

    pub fn append_child(parent: ElementId, child: ElementId) {
        STATE.with(|s| s.borrow_mut().append_child(parent, child))
    }

    pub fn on_click(el: ElementId, handler_id: u64) {
        STATE.with(|s| s.borrow_mut().on_click(el, handler_id))
    }
}

impl Default for RuntimeStub {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkGuest for RuntimeStub {
    fn fetch(req: Request) -> Result<Response, String> {
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
    fn create_element(tag: String) -> ElementId {
        Self::create_element(tag)
    }

    fn set_text(el: ElementId, text: String) {
        Self::set_text(el, text)
    }

    fn set_attribute(el: ElementId, name: String, value: String) {
        Self::set_attribute(el, name, value)
    }

    fn append_child(parent: ElementId, child: ElementId) {
        Self::append_child(parent, child)
    }

    fn on_click(el: ElementId, handler_id: u64) {
        Self::on_click(el, handler_id)
    }
}
