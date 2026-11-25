//! A click on the render surface resolves to the guest element under the pointer,
//! and that element's handlers are the ones the host dispatches.

use tpt_helix_appfront::HelixDocument;
use tpt_helix_runtime::stub::RuntimeState;

#[test]
fn a_click_on_a_guest_button_reports_its_element_and_handlers() {
    let mut guest = RuntimeState::default();
    let root = guest.create_element("div".into());
    let button = guest.create_element("button".into());
    guest.set_text(button, "Save".into());
    guest.set_attribute(
        button,
        "style".into(),
        "width: 120px; height: 40px; background: #3366ff".into(),
    );
    guest.append_child(root, button);
    guest.on_click(button, 7);

    let html = guest.document_html();
    let frame = HelixDocument::parse(&html).frame(800.0, 600.0);
    let hit = frame
        .hits
        .iter()
        .find(|h| h.tag == "button")
        .expect("the button is laid out");
    let (x, y) = (hit.x + hit.w / 2.0, hit.y + hit.h / 2.0);

    // The text inside the button is a box of its own. The click still resolves to
    // the button, the element that contains it.
    assert_eq!(frame.element_at(x, y), Some(button));
    assert_eq!(guest.click_handler_ids(button), Some(vec![7]));
}
