//! A click on a guest's page runs the guest's handler, through the page's own
//! elements, as DOM events bubble. The guest is the componentized guest example.

use tpt_helix_appfront::{GuestSurface, HelixDocument};
use tpt_helix_runtime::capability::{Capability, DomScope, HostPattern, StorageScope};

const HELLO_DOM: &[u8] = include_bytes!("../../tpt-helix-runtime/tests/hello_dom.wasm");

fn grants() -> Vec<Capability> {
    vec![
        Capability::Dom {
            scope: DomScope::Full,
        },
        Capability::Storage {
            scope: StorageScope::Global,
        },
        Capability::Network {
            hosts: vec![HostPattern::Any],
        },
    ]
}

#[test]
fn a_click_on_the_guest_text_bubbles_to_its_handler() {
    let mut guest = GuestSurface::load(HELLO_DOM, "hello", grants()).expect("loads and runs");
    let html = guest.html();
    let frame = HelixDocument::parse(&html).frame(800.0, 600.0);

    // The guest's handler is on the `div`, and the click lands on the text inside
    // its `span` child. The span has no handler, so the click bubbles to the div.
    let text = frame
        .hits
        .iter()
        .find(|h| h.tag == "#text")
        .expect("the guest's text is laid out");
    let element = frame
        .element_at(text.x + 1.0, text.y + 1.0)
        .expect("the text belongs to a guest element");

    let page = guest.click(element).expect("dispatch");
    assert!(page.is_some(), "a handler ran");
    assert_eq!(
        guest.host().stored("clicked").as_deref(),
        Some(&b"yes"[..]),
        "the div's click handler (id 1) stored its flag"
    );
}

#[test]
fn a_click_on_an_element_with_no_handler_changes_nothing() {
    let mut guest = GuestSurface::load(HELLO_DOM, "hello", grants()).expect("loads and runs");
    let page = guest.click(u64::MAX).expect("no handlers is not an error");
    assert!(page.is_none());
    assert!(guest.host().stored("clicked").is_none());
}
