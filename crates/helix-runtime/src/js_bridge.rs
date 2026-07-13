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

use crate::stub::RuntimeStub;
use helix_wit::host::exports::helix::runtime::dom::ElementId as WitElementId;
use helix_wit::host::exports::helix::runtime::network::Request as WitRequest;

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

/// Installs every bridge surface (dom + storage + network) at once.
pub fn install_all(ctx: Ctx<'_>) -> JsResult<()> {
    install_dom_bridge(ctx.clone())?;
    install_storage_bridge(ctx.clone())?;
    install_network_bridge(ctx.clone())?;
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
        interpreter.with(|ctx| install_dom_bridge(ctx)).unwrap();

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
        interpreter.with(|ctx| install_dom_bridge(ctx)).unwrap();

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
        interpreter.with(|ctx| install_storage_bridge(ctx)).unwrap();

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
        interpreter.with(|ctx| install_network_bridge(ctx)).unwrap();

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
}
