//! Headless validation of the AppFront `egui` ↔ `taffy` bridge.
//!
//! These tests run the layout + render pipeline without a window, asserting
//! that parsed HTML/CSS produces the expected `taffy` layout and render items.

use appfront::HelixDocument;
use appfront::render::RenderItem;

fn rects(items: &[RenderItem]) -> impl Iterator<Item = &appfront::render::RectItem> {
    items.iter().filter_map(|i| match i {
        RenderItem::Rect(r) => Some(r),
        _ => None,
    })
}

fn texts(items: &[RenderItem]) -> impl Iterator<Item = &appfront::render::TextItem> {
    items.iter().filter_map(|i| match i {
        RenderItem::Text(t) => Some(t),
        _ => None,
    })
}

#[test]
fn renders_static_html_into_rects_and_text() {
    let doc = HelixDocument::parse(
        r#"<html><body>
             <div style="background:#ff0000; width:100px; height:40px;"></div>
             <p>Hello</p>
           </body></html>"#,
    );
    let items = doc.render(800.0, 600.0);

    let reds = rects(&items).filter(|r| r.fill == [255, 0, 0, 255]).count();
    assert!(reds >= 1, "expected a red rect from inline style");

    let hello = texts(&items).any(|t| t.text == "Hello");
    assert!(hello, "expected a 'Hello' text run");
}

#[test]
fn stylesheet_rules_apply_to_matching_elements() {
    let doc = HelixDocument::parse(
        r#"<style>.card { background:#123456; }</style>
           <div class="card" style="width:50px; height:20px;"></div>"#,
    );
    let items = doc.render(800.0, 600.0);
    let cards = rects(&items)
        .filter(|r| r.fill == [0x12, 0x34, 0x56, 255])
        .count();
    assert_eq!(cards, 1, "stylesheet .card rule should color the div");
}

#[test]
fn block_elements_stack_vertically() {
    let doc = HelixDocument::parse(
        r#"<div style="background:#00ff00; width:100px; height:30px;"></div>
           <div style="background:#0000ff; width:100px; height:30px;"></div>"#,
    );
    let items = doc.render(800.0, 600.0);
    let greens: Vec<_> = rects(&items)
        .filter(|r| r.fill == [0, 255, 0, 255])
        .cloned()
        .collect();
    let blues: Vec<_> = rects(&items)
        .filter(|r| r.fill == [0, 0, 255, 255])
        .cloned()
        .collect();
    assert_eq!(greens.len(), 1);
    assert_eq!(blues.len(), 1);
    assert_eq!(greens[0].y, 0.0, "first block at top");
    assert!(blues[0].y > greens[0].y, "second block stacks below first");
    assert_eq!(blues[0].y, 30.0, "stacked by first block's height");
}

#[test]
fn border_and_radius_surface_in_render_item() {
    let doc = HelixDocument::parse(
        r#"<div style="background:#ffffff; width:80px; height:40px; border:3px solid #000000; border-radius:6px;"></div>"#,
    );
    let items = doc.render(800.0, 600.0);
    let r = rects(&items)
        .into_iter()
        .find(|r| r.fill == [255, 255, 255, 255])
        .expect("white box present");
    assert_eq!(r.border_width, 3.0);
    assert_eq!(r.border_color, Some([0, 0, 0, 255]));
    assert_eq!(r.radius, 6.0);
}

#[test]
fn tree_has_at_least_root_and_one_node() {
    let doc = HelixDocument::parse(r#"<p>x</p>"#);
    let built = doc.build(800.0, 600.0);
    assert!(built.node_count() >= 2, "root + content node");
}
