//! Headless tests for hit testing and the per-size frame cache. These need no
//! window: they exercise the layout pass that the `egui` shell paints and
//! routes clicks through.

use tpt_helix_appfront::{FrameCache, HelixDocument};

#[test]
fn hit_test_returns_the_innermost_box() {
    let doc = HelixDocument::parse(
        r#"<div style="width:200px; height:100px; background:#00ff00;">
             <div style="width:50px; height:20px; background:#ff0000;"></div>
           </div>"#,
    );
    let frame = doc.frame(800.0, 600.0);

    // Inside the red child: the child (50px wide) wins over its parent.
    let inner = frame.hit_test(10.0, 10.0).expect("child should be hit");
    assert_eq!(inner.tag, "div");
    assert_eq!(inner.w, 50.0);

    // Inside the parent but outside the child: the parent (200px wide) is hit.
    let outer = frame.hit_test(120.0, 50.0).expect("parent should be hit");
    assert_eq!(outer.w, 200.0);
    assert!(outer.node < inner.node, "pre-order: parent precedes child");
}

#[test]
fn hit_test_reaches_elements_without_a_background() {
    // A paragraph with no background paints nothing, but it is still clickable.
    let doc = HelixDocument::parse(r#"<p style="width:120px; height:24px;">Press me</p>"#);
    let frame = doc.frame(800.0, 600.0);

    assert!(
        frame
            .items
            .iter()
            .all(|i| !matches!(i, tpt_helix_appfront::render::RenderItem::Rect(_))),
        "no background means no rect is painted"
    );
    // The innermost box at the point is the text run inside the paragraph. The
    // paragraph itself is still a hit box (it precedes its text in pre-order).
    let inner = frame
        .hit_test(5.0, 5.0)
        .expect("text inside <p> should be hit");
    assert_eq!(inner.tag, "#text");
    let para = frame
        .hits
        .iter()
        .find(|b| b.tag == "p")
        .expect("unpainted <p> still has a hit box");
    assert_eq!(para.w, 120.0);
    assert!(para.node < inner.node, "the paragraph precedes its text");
}

#[test]
fn hit_test_misses_outside_every_box() {
    let doc =
        HelixDocument::parse(r#"<div style="width:40px; height:40px; background:#123456;"></div>"#);
    let frame = doc.frame(800.0, 600.0);
    assert!(frame.hit_test(500.0, 500.0).is_none());
}

#[test]
fn frame_cache_lays_out_only_when_the_size_changes() {
    let doc =
        HelixDocument::parse(r#"<div style="width:10px; height:10px; background:#000000;"></div>"#);
    let mut cache = FrameCache::new();

    let first = cache.frame(&doc, 800.0, 600.0).clone();
    let again = cache.frame(&doc, 800.0, 600.0).clone();
    assert_eq!(first, again);
    assert_eq!(cache.layouts(), 1, "same size reuses the cached frame");

    cache.frame(&doc, 640.0, 480.0);
    assert_eq!(cache.layouts(), 2, "a new size triggers one more layout");
}
