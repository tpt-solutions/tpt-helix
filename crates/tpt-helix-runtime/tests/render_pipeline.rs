//! End-to-end integration test for the static render pipeline:
//! HTML → CSS → layout → display-list → software raster → diff.
//!
//! This is the headless stand-in for a golden-file fixture: it drives the same
//! stages the GPU presenter does, but paints into an in-memory RGBA buffer so
//! it can run in CI without a display, and asserts on concrete pixel output.

use image::Rgba;
use tpt_helix_runtime::css::parse_stylesheet;
use tpt_helix_runtime::display_list::{build_display_list, build_text_runs};
use tpt_helix_runtime::html::parse_html;
use tpt_helix_runtime::layout::{build_layout_tree, compute};
use tpt_helix_runtime::screenshot_diff::{changed_bounds, compare};
use tpt_helix_runtime::software_raster::{rasterize_display_list, rasterize_text};
use tpt_helix_runtime::text::measure_text;

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
        tpt_helix_runtime::display_list::Color {
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
    assert!(
        bounds.0 >= 80 && bounds.2 <= 190,
        "change within col-b x: {bounds:?}"
    );
    assert!(
        bounds.1 >= 40 && bounds.3 <= 150,
        "change within col-b y: {bounds:?}"
    );
}

#[test]
fn background_shorthand_paints_like_background_color() {
    let raw = pipeline(
        r#"<html><body><div class="box"></div></body></html>"#,
        "div.box { width: 100px; height: 100px; background: #ff0000; }",
        400,
        300,
    );
    let img = image::RgbaImage::from_raw(400, 300, raw).expect("image");
    assert_eq!(img.get_pixel(50, 50), &Rgba([255, 0, 0, 255]));
}

#[test]
fn inline_style_paints_and_overrides_the_stylesheet() {
    let raw = pipeline(
        r#"<html><body><div class="box" style="background: #00ff00; width: 100px; height: 100px"></div></body></html>"#,
        "div.box { width: 100px; height: 100px; background-color: #ff0000; }",
        400,
        300,
    );
    let img = image::RgbaImage::from_raw(400, 300, raw).expect("image");
    assert_eq!(img.get_pixel(50, 50), &Rgba([0, 255, 0, 255]));
}

/// Like [`pipeline`], but also paints text runs, and returns the runs too.
fn pipeline_with_text(
    html: &str,
    css: &str,
    width: u32,
    height: u32,
) -> (
    image::RgbaImage,
    Vec<tpt_helix_runtime::display_list::TextRun>,
) {
    let dom = parse_html(html);
    let rules = parse_stylesheet(css);
    let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
    compute(&mut layout, width as f32, height as f32).expect("compute layout");
    let items = build_display_list(&layout.tree, layout.root, &rules);
    let runs = build_text_runs(&layout.tree, layout.root, &rules);
    let mut image = rasterize_display_list(&items, width, height);
    rasterize_text(&mut image, &runs);
    (image, runs)
}

/// Whether this machine has fonts. Text tests need them, and skip without.
fn fonts_available() -> bool {
    let (width, _) = measure_text("Hello", 16.0);
    if width == 0.0 {
        eprintln!("skipping text test: no system fonts");
    }
    width > 0.0
}

#[test]
fn text_is_painted_in_its_color() {
    if !fonts_available() {
        return;
    }
    let (img, runs) = pipeline_with_text(
        "<html><body><p>Hello</p></body></html>",
        "p { color: #ff0000; font-size: 24px; }",
        400,
        200,
    );
    assert_eq!(runs.len(), 1, "one text run for the paragraph");
    assert_eq!(runs[0].text, "Hello");
    assert_eq!(runs[0].font_size, 24.0);
    let painted = img
        .pixels()
        .filter(|p| p.0[3] > 0 && p.0[0] > 200 && p.0[1] < 60 && p.0[2] < 60)
        .count();
    assert!(painted > 20, "red glyph pixels expected, found {painted}");
}

#[test]
fn a_block_of_only_text_takes_its_height_so_its_background_paints() {
    if !fonts_available() {
        return;
    }
    // Before text had a size, a body holding only text laid out to 0x0 and its
    // background was never painted.
    let (img, _) = pipeline_with_text(
        "<html><body><p>Hi</p></body></html>",
        "body { background-color: #ff0000; }",
        400,
        200,
    );
    assert_eq!(img.get_pixel(2, 2), &Rgba([255, 0, 0, 255]));
}

#[test]
fn head_contents_are_not_painted_as_text() {
    if !fonts_available() {
        return;
    }
    let (_, runs) = pipeline_with_text(
        "<html><head><title>Secret</title><style>p { color: red }</style></head><body><p>Shown</p></body></html>",
        "",
        400,
        200,
    );
    let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["Shown"],
        "title and style bodies stay out of the page"
    );
}

#[test]
fn text_inherits_color_and_font_size_from_ancestors() {
    if !fonts_available() {
        return;
    }
    let (_, runs) = pipeline_with_text(
        r#"<html><body><div style="color: #00ff00; font-size: 20px"><p>x</p></div></body></html>"#,
        "",
        400,
        200,
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].font_size, 20.0);
    assert_eq!(runs[0].color.g, 1.0);
    assert_eq!(runs[0].color.r, 0.0);
}
