//! Every example under `examples/` runs as a test.
//!
//! * The static example renders headlessly to a PNG.
//! * Each pattern example is detected as its pattern and migrates to a guest.
//! * The legacy JS app's migration must reproduce the committed `after/` crate,
//!   so the before/after pair cannot drift from the generator.

use std::path::{Path, PathBuf};

use tpt_helix_cli::{migrate_html, render_html};
use tpt_helix_migrate::coverage::Pattern;
use tpt_helix_migrate::eve::detect_pattern;

fn example_dir(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(rel)
}

/// Reads a file with line endings normalized to LF. A Windows checkout may store
/// the committed golden files with CRLF, and the generator always writes LF.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Asserts that `dir/index.html` (with optional `dir/app.js`) is detected as
/// `pattern` and migrates to a guest.
fn assert_example_migrates(dir: &str, pattern: Pattern) {
    let root = example_dir(dir);
    let html = read(&root.join("index.html"));
    let js_path = root.join("app.js");
    let js = js_path.exists().then(|| read(&js_path));
    assert_eq!(
        detect_pattern(&html).0,
        pattern,
        "{dir} should be detected as {pattern:?}"
    );
    let name = root.file_name().unwrap().to_string_lossy().to_string();
    let (_, guest) = migrate_html(&name, &html, js.as_deref());
    assert!(guest.is_some(), "{dir} should produce a guest crate");
}

#[test]
fn hello_static_renders_to_png() {
    let html = read(&example_dir("hello-static/index.html"));
    let png = render_html(&html, "", 800, 600).expect("render hello-static");
    assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G'], "output is a PNG");
}

#[test]
fn hello_static_migrates_as_p1() {
    assert_example_migrates("hello-static", Pattern::P1StaticSite);
}

#[test]
fn todo_crud_migrates_as_p2() {
    assert_example_migrates("todo-crud", Pattern::P2FormCrud);
}

#[test]
fn chart_dashboard_migrates_as_p3() {
    assert_example_migrates("chart-dashboard", Pattern::P3DataViz);
}

#[test]
fn video_player_migrates_as_p4() {
    assert_example_migrates("video-player", Pattern::P4MediaPlayer);
}

#[test]
fn legacy_js_app_migration_matches_committed_output() {
    let before = example_dir("legacy-js-app/before");
    let html = read(&before.join("index.html"));
    let js = read(&before.join("app.js"));
    let (_, guest) = migrate_html("legacy-js-app", &html, Some(&js));
    let guest = guest.expect("the legacy app should produce a guest crate");

    let after = example_dir("legacy-js-app/after");
    assert_eq!(
        read(&after.join("Cargo.toml")),
        guest.cargo_toml.replace("\r\n", "\n"),
        "Cargo.toml drifted"
    );
    assert_eq!(
        read(&after.join("src/lib.rs")),
        guest.lib_rs.replace("\r\n", "\n"),
        "src/lib.rs drifted"
    );
    assert_eq!(
        read(&after.join("wit/helix.wit")),
        guest.helix_wit.replace("\r\n", "\n"),
        "wit/helix.wit drifted"
    );
}
