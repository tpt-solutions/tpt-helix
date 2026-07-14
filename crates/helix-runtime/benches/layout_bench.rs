//! Performance regression benchmark for the static render pipeline.
//!
//! Tracks the spec §7 throughput target:
//! * §7.3 Throughput — "Layout operations/sec: ≥60fps for 1000-element tree",
//!   i.e. a full `taffy` layout pass over 1000 elements must complete within
//!   one frame budget (~16.6 ms at 60 fps).
//!
//! Run with `cargo bench -p helix-runtime`. The criterion harness records
//! layout throughput over time so regressions are visible in CI trend charts.
//! The hard §7.3 budget is enforced as a failing test in
//! `tests/perf_regression.rs` (see `layout_1000_elements_meets_60fps_budget`).

use criterion::{Criterion, criterion_group, criterion_main};
use helix_runtime::css::parse_stylesheet;
use helix_runtime::html::parse_html;
use helix_runtime::layout::{build_layout_tree, compute};

/// Builds an N-element document: a `<body>` containing `n` flat `<div>`s, each
/// a flex container with a single child so the layout tree has real work to do.
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

fn bench_layout(c: &mut Criterion) {
    let n = 1000;
    c.bench_function("layout_1000_element_tree", |b| {
        b.iter(|| {
            let dom = parse_html(&make_document(n));
            let rules = parse_stylesheet(STYLESHEET);
            let mut layout = build_layout_tree(&dom, &rules).expect("layout tree builds");
            compute(&mut layout, 1280.0, 800.0).expect("layout computes")
        })
    });
}

criterion_group!(benches, bench_layout);
criterion_main!(benches);
