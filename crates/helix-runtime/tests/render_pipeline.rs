//! End-to-end integration test for the static render pipeline:
//! HTML → CSS → layout → display-list → software raster → diff.
//!
//! This is the headless stand-in for a golden-file fixture: it drives the same
//! stages the GPU presenter does, but paints into an in-memory RGBA buffer so
//! it can run in CI without a display, and asserts on concrete pixel output.

use helix_runtime::css::parse_stylesheet;
use helix_runtime::display_list::build_display_list;
use helix_runtime::html::parse_html;
use helix_runtime::layout::{build_layout_tree, compute};
use helix_runtime::screenshot_diff::{changed_bounds, compare};
use helix_runtime::software_raster::rasterize_display_list;
use image::Rgba;

/// Build the display list for `html`/`css` at a `width`x`height` viewport.
fn pipeline(html: &str, css: &str, width: u32, height: u32) -> Vec<u8> {
    let dom = parse_html(html);
    let rules = parse_stylesheet(css);
    let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
    compute(&mut layout, width as f32, height as f32).expect("compute layout");
    let items = build_display_list(&layout.tree, layout.root, &rules);
    let img = rasterize_display_list(&items, width, height);
    img.into_raw()
}

#[test]
fn paints_a_colored_box_and_leaves_background_transparent() {
    let raw = pipeline(
        r#"<html><body><div class="box"></div></body></html>"#,
        "div.box { width: 100px; height: 100px; background-color: #ff0000; }",
        400,
        300,
    );
    let img = image::RgbaImage::from_raw(400, 300, raw).expect("image");

    // A pixel well inside the 100x100 red box is opaque red.
    assert_eq!(img.get_pixel(50, 50), &Rgba([255, 0, 0, 255]));
    // A pixel outside the box stays transparent black.
    assert_eq!(img.get_pixel(200, 200), &Rgba([0, 0, 0, 0]));
}

#[test]
fn only_elements_with_background_produce_paint() {
    // Two divs, only one colored: the display list must contain exactly one
    // paint item.
    let html = r#"<html><body><div class="a"></div><div class="b"></div></body></html>"#;
    let css = "div.a { width: 10px; height: 10px; background-color: #ff0000; } \
               div.b { width: 20px; height: 20px; }";
    let dom = parse_html(html);
    let rules = parse_stylesheet(css);
    let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
    compute(&mut layout, 800.0, 600.0).expect("compute layout");
    let items = build_display_list(&layout.tree, layout.root, &rules);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].color,
        helix_runtime::display_list::Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0
        }
    );
}

#[test]
fn identical_pipeline_is_pixel_stable_against_baseline() {
    let html = r#"<html><body><div class="box"></div></body></html>"#;
    let css = "div.box { width: 60px; height: 60px; background-color: #00ff00; }";

    let a = pipeline(html, css, 200, 200);
    let b = pipeline(html, css, 200, 200);

    let a_img = image::RgbaImage::from_raw(200, 200, a).unwrap();
    let b_img = image::RgbaImage::from_raw(200, 200, b).unwrap();
    // Re-running the identical pipeline must be byte-for-byte identical.
    assert_eq!(compare(&a_img, &b_img).unwrap().changed_ratio, 0.0);
}

#[test]
fn changed_color_is_flagged_as_a_visual_regression() {
    let html = r#"<html><body><div class="box"></div></body></html>"#;
    let green = "div.box { width: 60px; height: 60px; background-color: #00ff00; }";
    let blue = "div.box { width: 60px; height: 60px; background-color: #0000ff; }";

    let green_raw = pipeline(html, green, 200, 200);
    let blue_raw = pipeline(html, blue, 200, 200);

    let green_img = image::RgbaImage::from_raw(200, 200, green_raw).unwrap();
    let blue_img = image::RgbaImage::from_raw(200, 200, blue_raw).unwrap();

    let report = compare(&green_img, &blue_img).unwrap();
    assert!(!report.identical);
    assert!(report.changed_ratio > 0.0);
    assert!(report.max_channel_delta > 0);
}

/// Beyond the single-box case: a composed multi-component layout (a header plus
/// two side-by-side columns) renders into distinct regions, and a regression
/// that recolors just one column is localized to that column's bounding box by
/// the visual-regression gate.
#[test]
fn composed_layout_regression_localizes_to_changed_region() {
    let html = r#"<html><body>
        <header class="hdr"></header>
        <div class="row">
            <div class="col col-a"></div>
            <div class="col col-b"></div>
        </div>
    </body></html>"#;
    let base = "\
        header.hdr { width: 200px; height: 40px; background-color: #333333; } \
        div.row { display: flex; } \
        div.col { width: 90px; height: 100px; } \
        div.col-a { background-color: #ff0000; } \
        div.col-b { background-color: #0000ff; }";
    // Regression: col-b is now green instead of blue.
    let regressed = "\
        header.hdr { width: 200px; height: 40px; background-color: #333333; } \
        div.row { display: flex; } \
        div.col { width: 90px; height: 100px; } \
        div.col-a { background-color: #ff0000; } \
        div.col-b { background-color: #00ff00; }";

    let base_raw = pipeline(html, base, 300, 200);
    let regr_raw = pipeline(html, regressed, 300, 200);
    let base_img = image::RgbaImage::from_raw(300, 200, base_raw).unwrap();
    let regr_img = image::RgbaImage::from_raw(300, 200, regr_raw).unwrap();

    // Header and col-a are untouched: the regression is confined to col-b.
    let bounds = changed_bounds(&regr_img, &base_img).expect("change present");
    // The flex row fills the 300px viewport; two 90px columns place col-b at
    // x in [90, 180), below the 40px header (y in [40, 140)).
    assert!(bounds.0 >= 80 && bounds.2 <= 190, "change within col-b x: {bounds:?}");
    assert!(bounds.1 >= 40 && bounds.3 <= 150, "change within col-b y: {bounds:?}");
}
