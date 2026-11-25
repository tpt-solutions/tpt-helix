//! Tests for the confidence report's screenshot check (spec §10.3). The check
//! renders the original page and the page the guest rebuilds, then compares
//! pixels. It must report a lost stylesheet and must not report a faithful
//! rebuild.

use tpt_helix_cli::{ScreenshotCheck, screenshot_check};
use tpt_helix_migrate::transpile::transpile_static_site;

fn check(html: &str) -> ScreenshotCheck {
    let site = transpile_static_site(html);
    screenshot_check(html, &site, 800, 600).expect("screenshot check")
}

#[test]
fn a_page_with_no_paint_is_identical_to_its_rebuild() {
    let c = check("<html><body><p>plain text</p></body></html>");
    assert_eq!(c.changed_ratio, 0.0);
}

#[test]
fn an_inline_style_survives_the_rebuild() {
    // The transpiler keeps `style` as an attribute, so the rebuilt page paints the
    // same box.
    let c = check(
        r#"<html><body><div style="background:#ff0000;width:100px;height:50px"></div></body></html>"#,
    );
    assert_eq!(c.changed_ratio, 0.0, "{c:?}");
}

#[test]
fn a_stylesheet_is_lost_in_the_rebuild_and_the_check_reports_it() {
    // The guest carries no `<style>` blocks, so the rebuilt page has no red box.
    let c =
        check("<style>div { background: #ff0000; width: 100px; height: 50px; }</style><div></div>");
    assert!(c.changed_ratio > 0.0, "{c:?}");
}
