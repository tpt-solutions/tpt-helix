//! Conformance tests for the Helix capability interfaces exercised through the
//! JS → WIT bridge (`crate::js_bridge`) instead of calling `RuntimeStub`
//! directly. This is the companion to `conformance.rs`: the same `dom` /
//! `storage` / `network` contract must hold whether a guest is a WASM module
//! or legacy JS running in QuickJS, because both delegate to the same
//! `RuntimeStub` host state.

use helix_runtime::js::Interpreter;
use helix_runtime::js_bridge::{
    install_dom_bridge, install_network_bridge, install_storage_bridge,
};
use helix_runtime::stub::RuntimeStub;
use helix_wit::host::exports::helix::runtime::dom::ElementId;
use helix_wit::host::exports::helix::runtime::network::Response;

/// Run `source` in a fresh QuickJS interpreter with all bridges installed,
/// against a freshly-reset host state. Returns the string coercion of the
/// script's final expression (e.g. an element id).
fn run_bridged(source: &str) -> String {
    let _stub = RuntimeStub::new();
    let interpreter = Interpreter::new().unwrap();
    interpreter
        .with(|ctx| {
            install_dom_bridge(ctx.clone())?;
            install_storage_bridge(ctx.clone())?;
            install_network_bridge(ctx.clone())
        })
        .unwrap();
    interpreter
        .eval_to_string(source)
        .expect("js eval")
        .unwrap_or_default()
}

fn el(id: u64) -> ElementId {
    ElementId { id }
}

#[test]
fn dom_create_element_assigns_unique_ids() {
    let out = run_bridged(
        "var a = __helix_create_element('div');
         var b = __helix_create_element('span');
         a + ',' + b;",
    );
    let mut parts = out.split(',');
    let a: u64 = parts.next().unwrap().parse().unwrap();
    let b: u64 = parts.next().unwrap().parse().unwrap();
    assert_ne!(a, b);
    assert_eq!(RuntimeStub::element(el(a)).unwrap().tag, "div");
    assert_eq!(RuntimeStub::element(el(b)).unwrap().tag, "span");
}

#[test]
fn dom_set_text_and_attribute() {
    let out = run_bridged(
        "var e = __helix_create_element('p');
         __helix_set_text(e, 'hi');
         __helix_set_attribute(e, 'class', 'lead');
         e;",
    );
    let id: u64 = out.trim().parse().unwrap();
    let node = RuntimeStub::element(el(id)).unwrap();
    assert_eq!(node.text, "hi");
    assert_eq!(
        node.attributes.get("class").map(String::as_str),
        Some("lead")
    );
}

#[test]
fn dom_append_child_builds_tree() {
    let out = run_bridged(
        "var parent = __helix_create_element('ul');
         var child = __helix_create_element('li');
         __helix_append_child(parent, child);
         parent;",
    );
    let parent_id: u64 = out.trim().parse().unwrap();
    let node = RuntimeStub::element(el(parent_id)).unwrap();
    assert_eq!(node.children.len(), 1);
    let child_id = node.children[0].id;
    assert_eq!(RuntimeStub::element(el(child_id)).unwrap().tag, "li");
}

#[test]
fn dom_on_click_registers_handlers() {
    let out = run_bridged(
        "var e = __helix_create_element('button');
         __helix_on_click(e, 7);
         __helix_on_click(e, 9);
         e;",
    );
    let id: u64 = out.trim().parse().unwrap();
    assert_eq!(RuntimeStub::click_count(el(id)), 2);
    assert_eq!(RuntimeStub::click_handler_ids(el(id)), Some(vec![7u64, 9]));
}

#[test]
fn storage_roundtrip_through_bridge() {
    run_bridged(
        "__helix_storage_set('k', 'v');
         __helix_storage_get('k');",
    );
    // The bridge wrote real host state; assert via the stub.
    assert_eq!(
        RuntimeStub::get("k".to_string()).as_deref(),
        Some(&b"v"[..])
    );
    run_bridged("__helix_storage_delete('k');");
    assert!(RuntimeStub::get("k".to_string()).is_none());
}

#[test]
fn network_fetch_through_bridge() {
    let _stub = RuntimeStub::new();
    _stub.register_fetch(
        "https://example.com/",
        Response {
            status: 200,
            headers: vec![],
            body: b"hello".to_vec(),
        },
    );
    let body = run_bridged("__helix_fetch('https://example.com/');");
    assert_eq!(body, "hello");
}
