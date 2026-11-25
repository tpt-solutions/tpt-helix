//! {{name}}: a Helix guest that fetches a URL and shows the result.
//!
//! The fetch succeeds only when the app is granted `network` (see `capabilities`).
//! Without that grant the runtime denies it, and the guest shows the error.

wit_bindgen::generate!({
    world: "helix-guest",
    path: "wit",
    additional_derives: [PartialEq, Eq, Hash],
});

use helix::runtime::{dom, network};

/// Entry point the host invokes after instantiating the component.
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    let status = dom::create_element(&"p");
    let request = network::Request {
        method: "GET".to_string(),
        url: "https://example.com/".to_string(),
        headers: vec![],
        body: None,
    };
    match network::fetch(&request) {
        Ok(response) => dom::set_text(status, &format!("fetched {} bytes", response.body.len())),
        Err(error) => dom::set_text(status, &format!("fetch failed: {error}")),
    }
}

/// Host callback for a handler registered with `on-click` or `on-event`.
#[unsafe(no_mangle)]
pub extern "C" fn dispatch(handler_id: u64) {
    let _ = handler_id;
}
