//! JS -> WIT bridge: exposes minimal host functions to QuickJS.
//!
//! Legacy JS running in [`crate::js::Interpreter`] has no built-in DOM; this
//! module is the seam that lets it reach the same `dom`, `storage` and `network`
//! capabilities every WASM guest uses. It installs global functions on a JS
//! context that delegate to a [`BridgeSession`].
//!
//! A session owns its own [`RuntimeState`] (elements, storage, fetch routes,
//! click handlers) and its own pending DOM batch. Nothing is shared between
//! sessions or with the thread-local [`crate::stub::RuntimeStub`] state, so two
//! legacy apps on one thread cannot see each other's DOM or storage. A session
//! built with [`BridgeSession::for_app`] routes every capability-checked call
//! through a [`CapabilityBroker`] and denies anything not granted to that app.

use rquickjs::{Ctx, Result as JsResult};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::capability::{AppId, Capability, CapabilityBroker, CapabilityToken};
use crate::stub::{Element, Request, RuntimeState};
use tpt_helix_wit::host::exports::helix::runtime::dom::ElementId as WitElementId;
use tpt_helix_wit::host::exports::helix::runtime::network::Response as WitResponse;

/// A pending DOM mutation recorded by the batched bridge before a `commit`.
///
/// Elements are referenced by a batch-local `handle` so a tree can be built up
/// across several `__helix_batch_*` calls without each one allocating a real
/// element id immediately.
#[derive(Debug, Clone)]
enum DomBatchOp {
    CreateElement {
        handle: u64,
        tag: String,
    },
    SetText {
        handle: u64,
        text: String,
    },
    SetAttribute {
        handle: u64,
        name: String,
        value: String,
    },
    AppendChild {
        parent: u64,
        child: u64,
    },
    OnClick {
        handle: u64,
        handler: u64,
    },
}

/// Largest number of ops one batch may accumulate before `commit`. Legacy JS
/// that exceeds it gets an error on commit instead of growing host memory
/// without bound (hostile-input hardening).
pub const MAX_BATCH_OPS: usize = 100_000;

/// A batch that exceeded [`MAX_BATCH_OPS`]. Batches are atomic: when this is
/// returned, none of the batch's ops were applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchOverflow {
    /// Ops that were rejected for lack of room.
    pub dropped: usize,
}

/// Accumulated batched DOM ops plus the next free batch-local handle.
#[derive(Default)]
struct DomBatch {
    next: u64,
    ops: Vec<DomBatchOp>,
    /// Ops rejected because `ops` was already at [`MAX_BATCH_OPS`].
    dropped: usize,
}

/// The isolated host state behind one legacy-JS app's bridge.
///
/// Cloning a session shares the same state (it is a handle, not a copy). Create
/// one session per app and pass it to every `install_*` call for that app's
/// context.
#[derive(Clone)]
pub struct BridgeSession {
    state: Rc<RefCell<RuntimeState>>,
    batch: Rc<RefCell<DomBatch>>,
}

impl BridgeSession {
    /// A session with no capability broker: every operation is permitted. This
    /// is the legacy path (mirrors `Host::unrestricted`) and exists for trusted
    /// in-process code and tests.
    pub fn unrestricted() -> Self {
        Self::with_state(RuntimeState::default())
    }

    /// A session for `app`, with every capability-checked operation enforced by
    /// `broker`. Nothing is granted until [`BridgeSession::grant`] is called.
    pub fn for_app(app: AppId, broker: CapabilityBroker) -> Self {
        Self::with_state(RuntimeState::with_broker(app, broker))
    }

    fn with_state(state: RuntimeState) -> Self {
        BridgeSession {
            state: Rc::new(RefCell::new(state)),
            batch: Rc::new(RefCell::new(DomBatch::default())),
        }
    }

    /// Grants `cap` to this session's app. Panics if the session has no broker.
    pub fn grant(&self, cap: Capability) -> CapabilityToken {
        self.state.borrow_mut().grant(cap)
    }

    /// Registers a canned fetch response for an exact URL.
    pub fn register_fetch(&self, url: impl Into<String>, response: WitResponse) {
        self.state.borrow_mut().register_fetch(
            url,
            crate::stub::Response {
                status: response.status,
                headers: response.headers,
                body: response.body,
            },
        );
    }

    /// Reads an element from this session's DOM.
    pub fn element(&self, id: WitElementId) -> Option<Element> {
        self.state.borrow().element(id.id)
    }

    /// Click handler ids registered on an element in this session.
    pub fn click_handler_ids(&self, id: WitElementId) -> Option<Vec<u64>> {
        self.state.borrow().click_handler_ids(id.id)
    }

    /// Creates an element directly in this session (not batched).
    pub fn create_element(&self, tag: String) -> WitElementId {
        WitElementId {
            id: self.state.borrow_mut().create_element(tag),
        }
    }

    /// The session's DOM as HTML, with each element's `data-helix-id`. See
    /// [`RuntimeState::document_html`].
    pub fn document_html(&self) -> String {
        self.state.borrow().document_html()
    }

    /// Discards any batched ops this session has not committed.
    pub fn clear_batch(&self) {
        *self.batch.borrow_mut() = DomBatch::default();
    }

    /// Records `op` in the pending batch, or counts it as dropped at the cap.
    fn push_op(&self, op: DomBatchOp) {
        let mut batch = self.batch.borrow_mut();
        if batch.ops.len() < MAX_BATCH_OPS {
            batch.ops.push(op);
        } else {
            batch.dropped += 1;
        }
    }

    /// Applies this session's batched ops to its state in one pass, then clears
    /// the buffer. Returns the number of ops applied, or [`BatchOverflow`]
    /// (applying nothing) if the batch exceeded [`MAX_BATCH_OPS`].
    ///
    /// Batch-local `handle`s are resolved to real element ids as the
    /// `CreateElement` ops are replayed, so later ops that reference an earlier
    /// handle observe the correct real id.
    pub fn commit_batch(&self) -> Result<usize, BatchOverflow> {
        let batch = std::mem::take(&mut *self.batch.borrow_mut());
        if batch.dropped > 0 {
            return Err(BatchOverflow {
                dropped: batch.dropped,
            });
        }
        let mut real_ids: HashMap<u64, u64> = HashMap::new();
        let count = batch.ops.len();
        for op in batch.ops {
            let mut st = self.state.borrow_mut();
            match op {
                DomBatchOp::CreateElement { handle, tag } => {
                    real_ids.insert(handle, st.create_element(tag));
                }
                DomBatchOp::SetText { handle, text } => {
                    if let Some(&id) = real_ids.get(&handle) {
                        st.set_text(id, text);
                    }
                }
                DomBatchOp::SetAttribute {
                    handle,
                    name,
                    value,
                } => {
                    if let Some(&id) = real_ids.get(&handle) {
                        st.set_attribute(id, name, value);
                    }
                }
                DomBatchOp::AppendChild { parent, child } => {
                    if let (Some(&p), Some(&c)) = (real_ids.get(&parent), real_ids.get(&child)) {
                        st.append_child(p, c);
                    }
                }
                DomBatchOp::OnClick { handle, handler } => {
                    if let Some(&id) = real_ids.get(&handle) {
                        st.on_click(id, handler);
                    }
                }
            }
        }
        Ok(count)
    }
}

/// Installs `__helix_*` global functions backed by `session` onto `ctx`. Names
/// are prefixed and left ungrouped (no `document.*` object) deliberately: this
/// is the low-level bridge surface, not the DOM API legacy apps see.
pub fn install_dom_bridge(ctx: Ctx<'_>, session: &BridgeSession) -> JsResult<()> {
    let globals = ctx.globals();
    let st = session.state.clone();

    globals.set(
        "__helix_create_element",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |tag: String| -> u64 { st.borrow_mut().create_element(tag) }
        }),
    )?;

    globals.set(
        "__helix_set_text",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |id: u64, text: String| st.borrow_mut().set_text(id, text)
        }),
    )?;

    globals.set(
        "__helix_set_attribute",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |id: u64, name: String, value: String| {
                st.borrow_mut().set_attribute(id, name, value)
            }
        }),
    )?;

    globals.set(
        "__helix_append_child",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |parent: u64, child: u64| st.borrow_mut().append_child(parent, child)
        }),
    )?;

    globals.set(
        "__helix_on_click",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |id: u64, handler_id: u64| st.borrow_mut().on_click(id, handler_id)
        }),
    )?;

    Ok(())
}

/// Installs `__helix_storage_*` bridge helpers backed by `session`.
///
/// These let legacy JS read/write the `storage` capability exactly as a WASM
/// guest would: same namespace checks, and the broker applies when the session
/// has one. Values are exchanged as UTF-8 strings for the bridge surface.
pub fn install_storage_bridge(ctx: Ctx<'_>, session: &BridgeSession) -> JsResult<()> {
    let globals = ctx.globals();
    let st = session.state.clone();

    globals.set(
        "__helix_storage_set",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |key: String, value: String| -> bool {
                st.borrow_mut().set(key, value.into_bytes()).is_ok()
            }
        }),
    )?;

    globals.set(
        "__helix_storage_get",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |key: String| -> Option<String> {
                st.borrow()
                    .get(key)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            }
        }),
    )?;

    globals.set(
        "__helix_storage_delete",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |key: String| -> bool { st.borrow_mut().delete(key).is_ok() }
        }),
    )?;

    Ok(())
}

/// Installs a `__helix_fetch` bridge helper backed by `session`.
///
/// The bridge surface issues a `GET` with no headers/body; legacy JS receives
/// the response body as a string (or `null` on error, no route, or denial).
pub fn install_network_bridge(ctx: Ctx<'_>, session: &BridgeSession) -> JsResult<()> {
    let globals = ctx.globals();
    let st = session.state.clone();

    globals.set(
        "__helix_fetch",
        rquickjs::Function::new(ctx.clone(), {
            let st = st.clone();
            move |url: String| -> Option<String> {
                let req = Request {
                    method: "GET".to_string(),
                    url,
                    headers: vec![],
                    body: None,
                };
                st.borrow()
                    .fetch(req)
                    .ok()
                    .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            }
        }),
    )?;

    Ok(())
}

/// `__helix_batch_commit` body: applies the batch, or throws a RangeError if it
/// exceeded [`MAX_BATCH_OPS`] (in which case nothing was applied).
fn commit_fn<'js>(session: BridgeSession) -> impl Fn(Ctx<'js>) -> JsResult<u32> + 'js {
    move |ctx: Ctx<'js>| match session.commit_batch() {
        Ok(applied) => Ok(applied as u32),
        Err(over) => Err(rquickjs::Exception::throw_range(
            &ctx,
            &format!(
                "__helix_batch_commit: batch exceeds {MAX_BATCH_OPS} ops ({} rejected); nothing applied",
                over.dropped
            ),
        )),
    }
}

/// Installs the batched DOM bridge (`__helix_batch_*` + `__helix_batch_commit`).
///
/// These helpers accumulate ops in the session's batch buffer and only touch the
/// session state once, on `commit`. That collapses the per-op borrow and
/// id-allocation overhead of building a large tree from legacy JS into a single
/// replay pass.
pub fn install_dom_batch_bridge(ctx: Ctx<'_>, session: &BridgeSession) -> JsResult<()> {
    let globals = ctx.globals();

    globals.set(
        "__helix_batch_create_element",
        rquickjs::Function::new(ctx.clone(), {
            let s = session.clone();
            move |tag: String| -> u64 {
                // Handles are always allocated; an over-cap create is dropped and
                // the batch then fails atomically on commit.
                let handle = {
                    let mut batch = s.batch.borrow_mut();
                    let handle = batch.next;
                    batch.next += 1;
                    handle
                };
                s.push_op(DomBatchOp::CreateElement { handle, tag });
                handle
            }
        }),
    )?;

    globals.set(
        "__helix_batch_set_text",
        rquickjs::Function::new(ctx.clone(), {
            let s = session.clone();
            move |handle: u64, text: String| s.push_op(DomBatchOp::SetText { handle, text })
        }),
    )?;

    globals.set(
        "__helix_batch_set_attribute",
        rquickjs::Function::new(ctx.clone(), {
            let s = session.clone();
            move |handle: u64, name: String, value: String| {
                s.push_op(DomBatchOp::SetAttribute {
                    handle,
                    name,
                    value,
                })
            }
        }),
    )?;

    globals.set(
        "__helix_batch_append_child",
        rquickjs::Function::new(ctx.clone(), {
            let s = session.clone();
            move |parent: u64, child: u64| s.push_op(DomBatchOp::AppendChild { parent, child })
        }),
    )?;

    globals.set(
        "__helix_batch_on_click",
        rquickjs::Function::new(ctx.clone(), {
            let s = session.clone();
            move |handle: u64, handler: u64| s.push_op(DomBatchOp::OnClick { handle, handler })
        }),
    )?;

    // A closure that captures only the session (no `Ctx`): a captured `Ctx`
    // stored in the context's own globals forms a cycle that trips QuickJS'
    // GC-list assertion when the runtime is torn down.
    globals.set(
        "__helix_batch_commit",
        rquickjs::Function::new(ctx.clone(), commit_fn(session.clone())),
    )?;

    Ok(())
}

/// Installs every bridge surface (dom + storage + network + dom-batch) for `session`.
pub fn install_all(ctx: Ctx<'_>, session: &BridgeSession) -> JsResult<()> {
    install_dom_bridge(ctx.clone(), session)?;
    install_storage_bridge(ctx.clone(), session)?;
    install_network_bridge(ctx.clone(), session)?;
    install_dom_batch_bridge(ctx.clone(), session)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js::Interpreter;

    #[test]
    fn js_can_create_and_mutate_an_element_through_the_bridge() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_bridge(c, &session))
            .unwrap();

        let element_id = interpreter
            .eval_to_string(
                "var el = __helix_create_element('div');
                 __helix_set_text(el, 'hello from JS');
                 __helix_set_attribute(el, 'class', 'greeting');
                 el;",
            )
            .unwrap()
            .expect("element id");

        let id: u64 = element_id.parse().unwrap();
        let element = session
            .element(WitElementId { id })
            .expect("element exists");
        assert_eq!(element.tag, "div");
        assert_eq!(element.text, "hello from JS");
        assert_eq!(
            element.attributes.get("class"),
            Some(&"greeting".to_string())
        );
    }

    #[test]
    fn js_can_build_a_parent_child_tree_and_register_a_click_handler() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_bridge(c, &session))
            .unwrap();

        let parent_id: u64 = interpreter
            .eval_to_string(
                "var parent = __helix_create_element('ul');
                 var child = __helix_create_element('li');
                 __helix_append_child(parent, child);
                 __helix_on_click(child, 7);
                 parent;",
            )
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();

        let parent = session.element(WitElementId { id: parent_id }).unwrap();
        assert_eq!(parent.children.len(), 1);
        let child_id = parent.children[0];
        assert_eq!(session.click_handler_ids(child_id), Some(vec![7]));
    }

    #[test]
    fn js_storage_bridge_roundtrips_through_stub() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_storage_bridge(c, &session))
            .unwrap();

        assert_eq!(
            interpreter
                .eval_to_string("__helix_storage_set('k', 'v')")
                .unwrap(),
            Some("true".to_string())
        );
        assert_eq!(
            interpreter
                .eval_to_string("__helix_storage_get('k')")
                .unwrap(),
            Some("v".to_string())
        );
        // Missing key maps to null in JS (None through the bridge).
        assert_eq!(
            interpreter
                .eval_to_string("__helix_storage_get('absent')")
                .unwrap(),
            None
        );
        assert_eq!(
            interpreter
                .eval_to_string("__helix_storage_delete('k')")
                .unwrap(),
            Some("true".to_string())
        );
        assert_eq!(
            interpreter
                .eval_to_string("__helix_storage_get('k')")
                .unwrap(),
            None
        );
    }

    #[test]
    fn js_network_bridge_fetches_registered_route() {
        let session = BridgeSession::unrestricted();
        session.register_fetch(
            "https://example.com/",
            tpt_helix_wit::host::exports::helix::runtime::network::Response {
                status: 200,
                headers: vec![],
                body: b"hello from net".to_vec(),
            },
        );
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_network_bridge(c, &session))
            .unwrap();

        let body = interpreter
            .eval_to_string("__helix_fetch('https://example.com/')")
            .unwrap()
            .expect("response body");
        assert_eq!(body, "hello from net");

        // Unregistered route yields null.
        assert_eq!(
            interpreter
                .eval_to_string("__helix_fetch('https://missing.test/')")
                .unwrap(),
            None
        );
    }

    #[test]
    fn js_batched_dom_bridge_builds_tree_on_commit() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_batch_bridge(c, &session))
            .unwrap();

        // Build a parent/child tree entirely in the buffer, then commit once.
        let parent_id: u64 = interpreter
            .eval_to_string(
                "var parent = __helix_batch_create_element('ul');
                 var child = __helix_batch_create_element('li');
                 __helix_batch_set_text(child, 'item');
                 __helix_batch_set_attribute(child, 'class', 'row');
                 __helix_batch_append_child(parent, child);
                 __helix_batch_on_click(child, 7);
                 var applied = __helix_batch_commit();
                 parent;",
            )
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();

        let parent = session.element(WitElementId { id: parent_id }).unwrap();
        assert_eq!(parent.tag, "ul");
        assert_eq!(parent.children.len(), 1);
        let child_id = parent.children[0];
        let child = session.element(child_id).unwrap();
        assert_eq!(child.tag, "li");
        assert_eq!(child.text, "item");
        assert_eq!(child.attributes.get("class"), Some(&"row".to_string()));
        assert_eq!(session.click_handler_ids(child_id), Some(vec![7]));
    }

    #[test]
    fn js_batched_dom_bridge_commit_returns_op_count() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_batch_bridge(c, &session))
            .unwrap();

        let applied: u32 = interpreter
            .eval_to_string(
                "var p = __helix_batch_create_element('div');
                 __helix_batch_set_text(p, 'x');
                 __helix_batch_set_attribute(p, 'id', 'a');
                 __helix_batch_commit();",
            )
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        // create + set_text + set_attribute = 3 ops replayed.
        assert_eq!(applied, 3);
    }

    #[test]
    fn js_batched_dom_bridge_is_noop_before_commit() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_batch_bridge(c, &session))
            .unwrap();

        // Accumulate ops but never commit; the host tree must stay empty.
        interpreter
            .eval_to_string(
                "var p = __helix_batch_create_element('div');
                 __helix_batch_set_text(p, 'x');",
            )
            .unwrap();
        let next = session.create_element("probe".to_string()).id;
        // Only the probe element exists (id 0); batched ops did not apply.
        assert_eq!(next, 0);
        session.clear_batch();
    }

    #[test]
    fn js_batch_over_cap_throws_and_applies_nothing() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_batch_bridge(c, &session))
            .unwrap();

        // The create op plus MAX_BATCH_OPS set_text ops is one op over the cap.
        let result = interpreter.eval_to_string(&format!(
            "var p = __helix_batch_create_element('div');
             for (var i = 0; i < {MAX_BATCH_OPS}; i++) __helix_batch_set_text(p, 'x');
             __helix_batch_commit();"
        ));
        assert!(result.is_err(), "over-cap batch must throw on commit");
        // Atomic: no op from the rejected batch reached the host tree.
        assert_eq!(session.create_element("probe".to_string()).id, 0);
        session.clear_batch();
    }

    #[test]
    fn batch_at_cap_still_commits() {
        let session = BridgeSession::unrestricted();
        let interpreter = Interpreter::new().unwrap();
        interpreter
            .with(|c| install_dom_batch_bridge(c, &session))
            .unwrap();

        let applied: u32 = interpreter
            .eval_to_string(&format!(
                "var p = __helix_batch_create_element('div');
                 for (var i = 0; i < {}; i++) __helix_batch_set_text(p, 'x');
                 __helix_batch_commit();",
                MAX_BATCH_OPS - 1
            ))
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(applied as usize, MAX_BATCH_OPS);
    }

    #[test]
    fn sessions_do_not_share_dom_or_storage() {
        let a = BridgeSession::unrestricted();
        let b = BridgeSession::unrestricted();
        let ia = Interpreter::new().unwrap();
        ia.with(|c| install_all(c, &a)).unwrap();
        let ib = Interpreter::new().unwrap();
        ib.with(|c| install_all(c, &b)).unwrap();

        let a_el: u64 = ia
            .eval_to_string(
                "__helix_storage_set('k', 'from-a');
                 __helix_create_element('div');",
            )
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();

        let seen = ib
            .eval_to_string("String(__helix_storage_get('k') == null)")
            .unwrap()
            .unwrap_or_default();
        assert_eq!(seen, "true", "session b must not read session a's storage");
        assert!(b.element(WitElementId { id: a_el }).is_none());
        assert!(a.element(WitElementId { id: a_el }).is_some());
    }

    #[test]
    fn brokered_session_denies_storage_until_granted() {
        let session = BridgeSession::for_app(
            "app:notes".to_string(),
            crate::capability::CapabilityBroker::new(),
        );
        let interp = Interpreter::new().unwrap();
        interp.with(|c| install_all(c, &session)).unwrap();

        let denied = interp
            .eval_to_string("String(__helix_storage_set('app:notes:k', 'v'))")
            .unwrap()
            .unwrap_or_default();
        assert_eq!(denied, "false", "ungranted app must be denied storage");

        session.grant(crate::capability::Capability::Storage {
            scope: crate::capability::StorageScope::Namespace("app:notes:".into()),
        });
        let allowed = interp
            .eval_to_string("String(__helix_storage_set('app:notes:k', 'v'))")
            .unwrap()
            .unwrap_or_default();
        assert_eq!(allowed, "true", "granted namespace must be writable");
    }
}
