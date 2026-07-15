//! Task: Define the JS -> WIT bridge stub (expose minimal host functions to
//! QuickJS).
//!
//! Legacy JS running in [`crate::js::Interpreter`] has no built-in DOM; this
//! module is the seam that lets it reach the same `dom` WIT capability every
//! WASM guest uses; it installs global functions on a JS context that
//! delegate straight to [`crate::stub::RuntimeStub`], so legacy JS and native
//! WASM modules observe the same host DOM state through one code path.
//! `network`/`storage` bridging follows the same shape once JS apps need it.

use rquickjs::{Ctx, Result as JsResult};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::stub::RuntimeStub;
use helix_wit::host::exports::helix::runtime::dom::ElementId as WitElementId;
use helix_wit::host::exports::helix::runtime::network::Request as WitRequest;

/// A pending DOM mutation recorded by the batched bridge before a `commit`.
///
/// Elements are referenced by a batch-local `handle` so a tree can be built up
/// across several `__helix_batch_*` calls without each one round-tripping
/// through [`RuntimeStub`] and allocating a real id immediately.
#[derive(Debug, Clone)]
enum DomBatchOp {
    CreateElement { handle: u64, tag: String },
    SetText { handle: u64, text: String },
    SetAttribute { handle: u64, name: String, value: String },
    AppendChild { parent: u64, child: u64 },
    OnClick { handle: u64, handler: u64 },
}

/// Accumulated batched DOM ops plus the next free batch-local handle.
#[derive(Default)]
struct DomBatch {
    next: u64,
    ops: Vec<DomBatchOp>,
}

thread_local! {
    /// Per-thread pending batched DOM ops (mirrors [`RuntimeStub`]'s
    /// thread-local host state).
    static DOM_BATCH: RefCell<DomBatch> = RefCell::new(DomBatch::default());
}

/// Applies the accumulated batched ops to [`RuntimeStub`] in one pass, then
/// clears the buffer. Returns the number of ops applied.
///
/// Batch-local `handle`s are resolved to real `RuntimeStub` element ids as the
/// `CreateElement` ops are replayed, so later ops in the batch that reference
/// an earlier handle observe the correct real id.
pub fn commit_dom_batch() -> usize {
    let batch = DOM_BATCH.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let mut real_ids: HashMap<u64, u64> = HashMap::new();
    let count = batch.ops.len();
    for op in batch.ops {
        match op {
            DomBatchOp::CreateElement { handle, tag } => {
                let real = RuntimeStub::create_element(tag).id;
                real_ids.insert(handle, real);
            }
            DomBatchOp::SetText { handle, text } => {
                if let Some(&id) = real_ids.get(&handle) {
                    RuntimeStub::set_text(WitElementId { id }, text);
                }
            }
            DomBatchOp::SetAttribute { handle, name, value } => {
                if let Some(&id) = real_ids.get(&handle) {
                    RuntimeStub::set_attribute(WitElementId { id }, name, value);
                }
            }
            DomBatchOp::AppendChild { parent, child } => {
                if let (Some(&p), Some(&c)) = (real_ids.get(&parent), real_ids.get(&child)) {
                    RuntimeStub::append_child(WitElementId { id: p }, WitElementId { id: c });
                }
            }
            DomBatchOp::OnClick { handle, handler } => {
                if let Some(&id) = real_ids.get(&handle) {
                    RuntimeStub::on_click(WitElementId { id }, handler);
                }
            }
        }
    }
    count
}

/// Discard any accumulated batched ops without applying them.
pub fn clear_dom_batch() {
    DOM_BATCH.with(|b| {
        *b.borrow_mut() = DomBatch::default();
    });
}

/// Installs `__helix_*` global functions backed by [`RuntimeStub`] onto
/// `ctx`. Names are prefixed and left ungrouped (no `document.*` object)
/// deliberately: this is the low-level bridge surface, not the DOM API
/// legacy apps see — a JS-side shim would build `document.createElement`
/// etc. on top of these once one exists.
pub fn install_dom_bridge(ctx: Ctx<'_>) -> JsResult<()> {
    let globals = ctx.globals();

    globals.set(
        "__helix_create_element",
        rquickjs::Function::new(ctx.clone(), |tag: String| -> u64 {
            RuntimeStub::create_element(tag).id
        }),
    )?;

    globals.set(
        "__helix_set_text",
        rquickjs::Function::new(ctx.clone(), |id: u64, text: String| {
            RuntimeStub::set_text(WitElementId { id }, text);
        }),
    )?;

    globals.set(
        "__helix_set_attribute",
        rquickjs::Function::new(ctx.clone(), |id: u64, name: String, value: String| {
            RuntimeStub::set_attribute(WitElementId { id }, name, value);
        }),
    )?;

    globals.set(
        "__helix_append_child",
        rquickjs::Function::new(ctx.clone(), |parent: u64, child: u64| {
            RuntimeStub::append_child(WitElementId { id: parent }, WitElementId { id: child });
        }),
    )?;

    globals.set(
        "__helix_on_click",
        rquickjs::Function::new(ctx.clone(), |id: u64, handler_id: u64| {
            RuntimeStub::on_click(WitElementId { id }, handler_id);
        }),
    )?;

    Ok(())
}

/// Installs `__helix_storage_*` bridge helpers delegating to [`RuntimeStub`].
///
/// These let legacy JS read/write the `storage` capability exactly as a WASM
/// guest would: same host state, same integrity/namespace semantics. Values are
/// exchanged as UTF-8 strings for the bridge surface.
pub fn install_storage_bridge(ctx: Ctx<'_>) -> JsResult<()> {
    let globals = ctx.globals();

    globals.set(
        "__helix_storage_set",
        rquickjs::Function::new(ctx.clone(), |key: String, value: String| -> bool {
            RuntimeStub::set(key, value.into_bytes()).is_ok()
        }),
    )?;

    globals.set(
        "__helix_storage_get",
        rquickjs::Function::new(ctx.clone(), |key: String| -> Option<String> {
            RuntimeStub::get(key).map(|b| String::from_utf8_lossy(&b).into_owned())
        }),
    )?;

    globals.set(
        "__helix_storage_delete",
        rquickjs::Function::new(ctx.clone(), |key: String| -> bool {
            RuntimeStub::delete(key).is_ok()
        }),
    )?;

    Ok(())
}

/// Installs a `__helix_fetch` bridge helper delegating to [`RuntimeStub::fetch`].
///
/// The bridge surface issues a `GET` with no headers/body; legacy JS receives
/// the response body as a string (or `null` on error / no route), mirroring the
/// `network` capability path a WASM guest uses.
pub fn install_network_bridge(ctx: Ctx<'_>) -> JsResult<()> {
    let globals = ctx.globals();

    globals.set(
        "__helix_fetch",
        rquickjs::Function::new(ctx.clone(), |url: String| -> Option<String> {
            let req = WitRequest {
                method: "GET".to_string(),
                url,
                headers: vec![],
                body: None,
            };
            RuntimeStub::fetch(req)
                .ok()
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        }),
    )?;

    Ok(())
}

/// Installs the batched DOM bridge (`__helix_batch_*` + `__helix_batch_commit`).
///
/// Unlike the per-call DOM bridge, these helpers accumulate ops in a thread-local
/// buffer and only touch [`RuntimeStub`] once, on `commit`. That collapses the
/// per-op thread-local borrow + id-allocation overhead of building a large tree
/// from legacy JS into a single replay pass (the "batching" efficiency work noted
/// in TODO.md §"Legacy JS compatibility layer optimization").
pub fn install_dom_batch_bridge(ctx: Ctx<'_>) -> JsResult<()> {
    let globals = ctx.globals();

    globals.set(
        "__helix_batch_create_element",
        rquickjs::Function::new(ctx.clone(), |tag: String| -> u64 {
            DOM_BATCH.with(|b| {
                let mut batch = b.borrow_mut();
                let handle = batch.next;
                batch.next += 1;
                batch.ops.push(DomBatchOp::CreateElement { handle, tag });
                handle
            })
        }),
    )?;

    globals.set(
        "__helix_batch_set_text",
        rquickjs::Function::new(ctx.clone(), |handle: u64, text: String| {
            DOM_BATCH.with(|b| {
                b.borrow_mut()
                    .ops
                    .push(DomBatchOp::SetText { handle, text });
            });
        }),
    )?;

    globals.set(
        "__helix_batch_set_attribute",
        rquickjs::Function::new(ctx.clone(), |handle: u64, name: String, value: String| {
            DOM_BATCH.with(|b| {
                b.borrow_mut()
                    .ops
                    .push(DomBatchOp::SetAttribute { handle, name, value });
            });
        }),
    )?;

    globals.set(
        "__helix_batch_append_child",
        rquickjs::Function::new(ctx.clone(), |parent: u64, child: u64| {
            DOM_BATCH.with(|b| {
                b.borrow_mut()
                    .ops
                    .push(DomBatchOp::AppendChild { parent, child });
            });
        }),
    )?;

    globals.set(
        "__helix_batch_on_click",
        rquickjs::Function::new(ctx.clone(), |handle: u64, handler: u64| {
            DOM_BATCH.with(|b| {
                b.borrow_mut()
                    .ops
                    .push(DomBatchOp::OnClick { handle, handler });
            });
        }),
    )?;

    globals.set(
        "__helix_batch_commit",
        rquickjs::Function::new(ctx.clone(), || -> u32 {
            commit_dom_batch() as u32
        }),
    )?;

    Ok(())
}

/// Installs every bridge surface (dom + storage + network + dom-batch) at once.
pub fn install_all(ctx: Ctx<'_>) -> JsResult<()> {
    install_dom_bridge(ctx.clone())?;
    install_storage_bridge(ctx.clone())?;
    install_network_bridge(ctx.clone())?;
    install_dom_batch_bridge(ctx.clone())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js::Interpreter;

    #[test]
    fn js_can_create_and_mutate_an_element_through_the_bridge() {
        let _stub = RuntimeStub::new(); // resets thread-local host state
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_dom_bridge).unwrap();

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
        let element = RuntimeStub
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
        let _stub = RuntimeStub::new();
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_dom_bridge).unwrap();

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

        let parent = RuntimeStub.element(WitElementId { id: parent_id }).unwrap();
        assert_eq!(parent.children.len(), 1);
        let child_id = parent.children[0];
        assert_eq!(RuntimeStub.click_handler_ids(child_id), Some(vec![7]));
    }

    #[test]
    fn js_storage_bridge_roundtrips_through_stub() {
        let _stub = RuntimeStub::new();
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_storage_bridge).unwrap();

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
        let stub = RuntimeStub::new();
        stub.register_fetch(
            "https://example.com/",
            helix_wit::host::exports::helix::runtime::network::Response {
                status: 200,
                headers: vec![],
                body: b"hello from net".to_vec(),
            },
        );
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_network_bridge).unwrap();

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
        let _stub = RuntimeStub::new();
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_dom_batch_bridge).unwrap();

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

        let parent = RuntimeStub.element(WitElementId { id: parent_id }).unwrap();
        assert_eq!(parent.tag, "ul");
        assert_eq!(parent.children.len(), 1);
        let child_id = parent.children[0];
        let child = RuntimeStub.element(child_id).unwrap();
        assert_eq!(child.tag, "li");
        assert_eq!(child.text, "item");
        assert_eq!(
            child.attributes.get("class"),
            Some(&"row".to_string())
        );
        assert_eq!(RuntimeStub.click_handler_ids(child_id), Some(vec![7]));
    }

    #[test]
    fn js_batched_dom_bridge_commit_returns_op_count() {
        let _stub = RuntimeStub::new();
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_dom_batch_bridge).unwrap();

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
        let _stub = RuntimeStub::new();
        let interpreter = Interpreter::new().unwrap();
        interpreter.with(install_dom_batch_bridge).unwrap();

        // Accumulate ops but never commit; the host tree must stay empty.
        interpreter
            .eval_to_string(
                "var p = __helix_batch_create_element('div');
                 __helix_batch_set_text(p, 'x');",
            )
            .unwrap();
        let next = RuntimeStub::create_element("probe".to_string()).id;
        // Only the probe element exists (id 0); batched ops did not apply.
        assert_eq!(next, 0);
        clear_dom_batch();
    }
}
