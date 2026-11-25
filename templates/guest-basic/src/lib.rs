//! {{name}}: a minimal Helix guest. The host calls `run` once after
//! instantiation, and `dispatch` once for each handler a DOM event triggers.
//!
//! Build: cargo build --target wasm32-unknown-unknown --release

wit_bindgen::generate!({
    world: "helix-guest",
    path: "wit",
    additional_derives: [PartialEq, Eq, Hash],
});

use helix::runtime::dom;

/// Entry point the host invokes after instantiating the component.
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    let root = dom::create_element(&"div");
    let title = dom::create_element(&"h1");
    dom::set_text(title, &"Hello from {{name}}");
    dom::append_child(root, title);
}

/// Host callback for a handler registered with `on-click` or `on-event`.
#[unsafe(no_mangle)]
pub extern "C" fn dispatch(handler_id: u64) {
    let _ = handler_id;
}
