//! Runtime stub implementing the generated host-side `Guest` traits.
//!
//! `RuntimeStub` stands in for the real Helix Runtime host until `wasmtime`
//! and the capability broker are wired up (see TODO.md §"Basic WASM module
//! loading" and §"Capability broker implementation").
//!
//! The host-side contract is *generated* by `wit_bindgen` from the `helix-runtime`
//! world in `wit/helix.wit` (see `crates/helix-wit`). Because that generated
//! `Guest` trait is stateless (its methods take no `&self` — they are the
//! component's exported entry points), the stub keeps its mutable state in a
//! thread-local `State` and `new()` resets it so conformance tests stay
//! isolated. When `wasmtime` is integrated, the generated `add_to_linker`
//! glues these `Guest` impls onto a component `Linker`, replacing the
//! hand-threaded state with the real component instance state.

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

/// Mutable host state, kept per-thread because the generated host traits are
/// stateless.
#[derive(Default)]
struct State {
    next_id: u64,
    elements: HashMap<u64, Element>,
    /// Click handler ids registered per element id.
    click_handlers: HashMap<u64, Vec<u64>>,
    store: HashMap<String, Vec<u8>>,
    /// Canned fetch responses keyed by exact request URL.
    fetch_responses: HashMap<String, Response>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// An in-memory stand-in for the Helix Runtime host.
///
/// `RuntimeStub` is a zero-sized marker for the host instance on the current
/// thread; all state lives in the thread-local `STATE` above.
pub struct RuntimeStub;

impl RuntimeStub {
    /// Creates a fresh host instance for the calling thread (resets state).
    pub fn new() -> Self {
        STATE.with(|s| *s.borrow_mut() = State::default());
        RuntimeStub
    }

    /// Registers a canned `fetch` response for an exact URL (test helper).
    pub fn register_fetch(&self, url: impl Into<String>, response: Response) {
        STATE.with(|s| {
            s.borrow_mut().fetch_responses.insert(url.into(), response);
        });
    }

    pub fn element(&self, id: ElementId) -> Option<Element> {
        STATE.with(|s| s.borrow().elements.get(&id.id).cloned())
    }

    pub fn click_count(&self, id: ElementId) -> usize {
        STATE.with(|s| s.borrow().click_handlers.get(&id.id).map(Vec::len).unwrap_or(0))
    }

    pub fn click_handler_ids(&self, id: ElementId) -> Option<Vec<u64>> {
        STATE.with(|s| s.borrow().click_handlers.get(&id.id).cloned())
    }
}

impl Default for RuntimeStub {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkGuest for RuntimeStub {
    fn fetch(req: Request) -> Result<Response, String> {
        STATE.with(|s| {
            s.borrow()
                .fetch_responses
                .get(&req.url)
                .cloned()
                .ok_or_else(|| format!("network capability denied or no route for {}", req.url))
        })
    }
}

impl StorageGuest for RuntimeStub {
    fn get(key: String) -> Option<Vec<u8>> {
        STATE.with(|s| s.borrow().store.get(&key).cloned())
    }

    fn set(key: String, value: Vec<u8>) -> Result<(), String> {
        STATE.with(|s| {
            s.borrow_mut().store.insert(key, value);
        });
        Ok(())
    }

    fn delete(key: String) -> Result<(), String> {
        STATE.with(|s| {
            s.borrow_mut().store.remove(&key);
        });
        Ok(())
    }
}

impl DomGuest for RuntimeStub {
    fn create_element(tag: String) -> ElementId {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            let id = st.next_id;
            st.next_id += 1;
            st.elements.insert(
                id,
                Element {
                    tag,
                    ..Default::default()
                },
            );
            ElementId { id }
        })
    }

    fn set_text(el: ElementId, text: String) {
        STATE.with(|s| {
            if let Some(node) = s.borrow_mut().elements.get_mut(&el.id) {
                node.text = text;
            }
        });
    }

    fn set_attribute(el: ElementId, name: String, value: String) {
        STATE.with(|s| {
            if let Some(node) = s.borrow_mut().elements.get_mut(&el.id) {
                node.attributes.insert(name, value);
            }
        });
    }

    fn append_child(parent: ElementId, child: ElementId) {
        STATE.with(|s| {
            if let Some(node) = s.borrow_mut().elements.get_mut(&parent.id) {
                node.children.push(child);
            }
        });
    }

    fn on_click(el: ElementId, handler_id: u64) {
        STATE.with(|s| {
            s.borrow_mut()
                .click_handlers
                .entry(el.id)
                .or_default()
                .push(handler_id);
        });
    }
}
