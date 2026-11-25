//! {{name}}: a Helix guest that remembers a value between runs.
//!
//! Storage reads and writes succeed only when the app is granted `storage` (see
//! `capabilities`). The key is namespaced by the app name.

wit_bindgen::generate!({
    world: "helix-guest",
    path: "wit",
    additional_derives: [PartialEq, Eq, Hash],
});

use helix::runtime::{dom, storage};

const KEY: &str = "{{name}}:greeting";

/// Entry point the host invokes after instantiating the component.
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    let label = dom::create_element(&"p");
    let greeting = match storage::get(&KEY) {
        Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        None => "first visit".to_string(),
    };
    dom::set_text(label, &greeting);
    let _ = storage::set(&KEY, b"welcome back".as_slice());
}

/// Host callback for a handler registered with `on-click` or `on-event`.
#[unsafe(no_mangle)]
pub extern "C" fn dispatch(handler_id: u64) {
    let _ = handler_id;
}
