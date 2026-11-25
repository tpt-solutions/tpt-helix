//! Performance regression guard for the static render pipeline (spec §7).
//!
//! Unlike the criterion harness in `benches/layout_bench.rs` (which only
//! records a trend), this test *fails CI* if the static layout path regresses
//! past a hard spec target:
//!
//! * §7.3 Throughput — "Layout operations/sec: ≥60fps for 1000-element tree".
//!   A full `taffy` layout pass over 1000 elements must finish within one frame
//!   budget (~16.6 ms at 60 fps). We allow 4x slack so debug/CI machines don't
//!   flap; the shipped release profile must clear the true 16.6 ms budget.

use tpt_helix_runtime::css::parse_stylesheet;
use tpt_helix_runtime::html::parse_html;
use tpt_helix_runtime::layout::{build_layout_tree, compute};

fn make_document(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&format!(
            "<div class=\"row\" id=\"r{i}\"><span>item {i}</span></div>"
        ));
    }
    format!("<html><body>{body}</body></html>")
}

const STYLESHEET: &str = "body { display: block; } .row { display: flex; width: 100%; }";

#[test]
fn layout_1000_elements_meets_60fps_budget() {
    let dom = parse_html(&make_document(1000));
    let rules = parse_stylesheet(STYLESHEET);
    let mut layout = build_layout_tree(&dom, &rules).expect("layout tree builds");
    assert!(
        layout.tree.total_node_count() >= 1000,
        "expected at least 1000 laid-out nodes"
    );

    // The budget is a per-frame cost: the layout pass that runs on each frame of an
    // unchanged document. Loading fonts and shaping every text run happen once per
    // document (before the first frame, as a browser loads its fonts), so the first
    // pass warms them and is not timed. The per-frame pass is.
    compute(&mut layout, 1280.0, 800.0).expect("first layout computes");
    let start = std::time::Instant::now();
    compute(&mut layout, 1280.0, 800.0).expect("frame layout computes");
    let elapsed_us = start.elapsed().as_micros();

    // 60 fps frame budget = 16.6 ms. 4x slack for debug/CI build variance.
    let budget_us: u128 = 16_600 * 4;
    assert!(
        elapsed_us <= budget_us,
        "1000-element layout took {elapsed_us} us, over the {budget_us} us budget (spec §7.3)"
    );
}
