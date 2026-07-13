//! Conformance tests for the Helix capability interfaces.
//!
//! These run against `RuntimeStub` — the in-memory stand-in for the real
//! Helix Runtime host (no `wasmtime` wired up yet). They assert that the
//! stub's behavior matches the WIT contract in `wit/helix.wit` by calling the
//! generated host-side `Guest` trait methods directly.

use helix_runtime::stub::RuntimeStub;
use helix_wit::host::exports::helix::runtime::media::VideoConfig;
use helix_wit::host::exports::helix::runtime::network::{Request, Response};
use helix_wit::host::exports::helix::runtime::{
    dom::Guest as _, media::Guest as _, network::Guest as _, storage::Guest as _,
};

fn req(url: &str) -> Request {
    Request {
        method: "GET".to_string(),
        url: url.to_string(),
        headers: vec![],
        body: None,
    }
}

// --- network -------------------------------------------------------------

#[test]
fn network_fetch_returns_registered_response() {
    let stub = RuntimeStub::new();
    stub.register_fetch(
        "https://example.com/",
        Response {
            status: 200,
            headers: vec![],
            body: b"hello".to_vec(),
        },
    );

    let res = RuntimeStub::fetch(req("https://example.com/")).expect("fetch ok");
    assert_eq!(res.status, 200);
    assert_eq!(res.body, b"hello");
}

#[test]
fn network_fetch_errors_for_unregistered_url() {
    let _stub = RuntimeStub::new();
    let err = RuntimeStub::fetch(req("https://missing.test/")).expect_err("no route");
    assert!(err.contains("missing.test"));
}

// --- storage -------------------------------------------------------------

#[test]
fn storage_roundtrip_set_get_delete() {
    let _stub = RuntimeStub::new();

    assert!(RuntimeStub::get("k".to_string()).is_none());

    RuntimeStub::set("k".to_string(), b"v".to_vec()).expect("set ok");
    assert_eq!(
        RuntimeStub::get("k".to_string()).as_deref(),
        Some(&b"v"[..])
    );

    RuntimeStub::delete("k".to_string()).expect("delete ok");
    assert!(RuntimeStub::get("k".to_string()).is_none());
}

#[test]
fn storage_delete_missing_key_is_ok() {
    let _stub = RuntimeStub::new();
    assert!(RuntimeStub::delete("absent".to_string()).is_ok());
}

// --- dom -----------------------------------------------------------------

#[test]
fn dom_create_element_assigns_unique_ids() {
    let _stub = RuntimeStub::new();
    let a = RuntimeStub::create_element("div".to_string());
    let b = RuntimeStub::create_element("span".to_string());
    assert_ne!(a.id, b.id);
    assert_eq!(_stub.element(a).unwrap().tag, "div");
    assert_eq!(_stub.element(b).unwrap().tag, "span");
}

#[test]
fn dom_set_text_and_attribute() {
    let stub = RuntimeStub::new();
    let el = RuntimeStub::create_element("p".to_string());
    RuntimeStub::set_text(el, "hi".to_string());
    RuntimeStub::set_attribute(el, "class".to_string(), "lead".to_string());

    let node = stub.element(el).unwrap();
    assert_eq!(node.text, "hi");
    assert_eq!(
        node.attributes.get("class").map(String::as_str),
        Some("lead")
    );
}

#[test]
fn dom_append_child_builds_tree() {
    let stub = RuntimeStub::new();
    let parent = RuntimeStub::create_element("ul".to_string());
    let child = RuntimeStub::create_element("li".to_string());
    RuntimeStub::append_child(parent, child);

    let node = stub.element(parent).unwrap();
    assert_eq!(node.children, vec![child]);
}

#[test]
fn dom_on_click_registers_handler() {
    let stub = RuntimeStub::new();
    let el = RuntimeStub::create_element("button".to_string());

    RuntimeStub::on_click(el, 7);
    RuntimeStub::on_click(el, 9);

    assert_eq!(stub.click_count(el), 2);
    assert_eq!(stub.click_handler_ids(el), Some(vec![7u64, 9]));
}

// --- media --------------------------------------------------------------

#[test]
fn media_player_lifecycle_via_stub() {
    let _stub = RuntimeStub::new();
    let handle = RuntimeStub::create_player(VideoConfig {
        codec: "h264".to_string(),
        width: 640,
        height: 480,
        bitrate: 1_000_000,
    })
    .expect("create player");

    assert!(RuntimeStub::player(handle).is_some());
    assert!(!RuntimeStub::player(handle).unwrap().playing);

    RuntimeStub::play(handle);
    assert!(RuntimeStub::player(handle).unwrap().playing);

    RuntimeStub::seek(handle, 5000);
    assert_eq!(RuntimeStub::player(handle).unwrap().position_ms, 5000);

    RuntimeStub::pause(handle);
    assert!(!RuntimeStub::player(handle).unwrap().playing);
}
