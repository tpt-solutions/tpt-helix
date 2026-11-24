//! Tests for the `helix` CLI: library behavior plus end-to-end runs of the
//! binary. Temporary directories live under the OS temp dir and are removed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tpt_helix_cli::{
    Grant, all_required_ok, doctor_checks, migrate_html, migration_report, migration_warnings,
    new_project, parse_grants, render_html, run_module, write_guest_crate,
};
use tpt_helix_runtime::software_raster::decode_png;

const HELLO_DOM: &[u8] = include_bytes!("../../tpt-helix-runtime/tests/hello_dom.wasm");

/// A unique, empty scratch directory that is removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "helix-cli-{tag}-{}-{nanos}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// --- render ----------------------------------------------------------------

#[test]
fn render_paints_the_inline_style_block() {
    let html = r#"<html><head><style>.box { width: 40px; height: 40px; background-color: #3366ff; }</style></head>
                  <body><div class="box"></div></body></html>"#;
    let png = render_html(html, "", 100, 100).expect("render");
    let img = decode_png(&png).expect("valid png");
    assert_eq!(img.get_pixel(10, 10).0, [51, 102, 255, 255]);
    assert_eq!(img.get_pixel(90, 90).0, [0, 0, 0, 0]);
}

#[test]
fn render_applies_extra_css_after_inline_styles() {
    let html = r#"<html><head><style>.box { width: 40px; height: 40px; background-color: #ff0000; }</style></head>
                  <body><div class="box"></div></body></html>"#;
    let png = render_html(html, ".box { background-color: #00ff00; }", 100, 100).unwrap();
    let img = decode_png(&png).unwrap();
    assert_eq!(img.get_pixel(10, 10).0, [0, 255, 0, 255]);
}

#[test]
fn render_rejects_out_of_range_viewports() {
    assert!(render_html("<p>x</p>", "", 0, 10).is_err());
    assert!(render_html("<p>x</p>", "", 10, 100_000).is_err());
}

// --- run -------------------------------------------------------------------

#[test]
fn parse_grants_accepts_known_names_and_rejects_unknown() {
    assert_eq!(
        parse_grants("dom, storage").unwrap(),
        vec![Grant::Dom, Grant::Storage]
    );
    assert_eq!(parse_grants("").unwrap(), Vec::<Grant>::new());
    assert!(parse_grants("dom,root").is_err());
}

#[test]
fn run_executes_a_guest_under_grants_and_unrestricted() {
    assert!(run_module(HELLO_DOM, "t", &[Grant::Dom, Grant::Storage], false).is_ok());
    assert!(run_module(HELLO_DOM, "t", &[], true).is_ok());
}

#[test]
fn run_rejects_bytes_that_are_not_a_component() {
    let err = run_module(b"not wasm", "t", &[], false).unwrap_err();
    assert!(err.to_string().contains("invalid module"), "{err}");
}

// --- migrate ---------------------------------------------------------------

#[test]
fn migration_report_names_pattern_and_stages() {
    let html = "<html><body><h1>Hello</h1><p>Static page.</p></body></html>";
    let (migration, _guest) = migrate_html("hello", html, None);
    let report = migration_report(&migration);
    assert!(report.contains("app: hello"), "{report}");
    assert!(report.contains("pattern:"), "{report}");
    assert!(report.contains("equivalence (S3):"), "{report}");
    assert!(report.contains("S5"), "{report}");
}

#[test]
fn migration_warnings_flag_unsupported_documents() {
    // Plain text with no recognizable structure matches no supported pattern.
    let (migration, _guest) = migrate_html("odd", "just some words", None);
    let warnings = migration_warnings(&migration);
    assert!(!warnings.is_empty());
}

#[test]
fn write_guest_crate_refuses_a_non_empty_directory() {
    let scratch = Scratch::new("guest");
    let (_migration, guest) = migrate_html("guest", "<p>hi</p>", None);
    let guest = guest.expect("a static page is supported");
    write_guest_crate(scratch.path(), &guest).expect("writes into empty dir");
    assert!(scratch.path().join("Cargo.toml").exists());
    assert!(scratch.path().join("src").join("lib.rs").exists());
    assert!(write_guest_crate(scratch.path(), &guest).is_err());
}

// --- new -------------------------------------------------------------------

#[test]
fn new_project_scaffolds_and_refuses_to_overwrite() {
    let scratch = Scratch::new("new");
    let app = scratch.path().join("my-app");
    new_project(&app).expect("scaffold");
    let index = std::fs::read_to_string(app.join("index.html")).unwrap();
    assert!(index.contains("<html"));
    assert!(new_project(&app).is_err());
}

// --- doctor ----------------------------------------------------------------

#[test]
fn doctor_reports_cargo_and_rustc_when_run_under_cargo() {
    // `cargo test` implies cargo and rustc are present.
    let checks = doctor_checks();
    let cargo = checks
        .iter()
        .find(|c| c.name == "cargo")
        .expect("cargo check");
    assert!(cargo.ok, "{cargo:?}");
    assert!(checks.iter().all(|c| !c.hint.is_empty()));
    let _ = all_required_ok(&checks);
}

// --- binary end to end -----------------------------------------------------

fn helix() -> Command {
    Command::new(env!("CARGO_BIN_EXE_helix"))
}

#[test]
fn binary_render_writes_a_png_next_to_the_input() {
    let scratch = Scratch::new("bin-render");
    let input = scratch.path().join("page.html");
    std::fs::write(
        &input,
        "<html><head><style>div { width: 10px; height: 10px; background-color: #ff0000; }</style></head><body><div></div></body></html>",
    )
    .unwrap();
    let status = helix()
        .arg("render")
        .arg(&input)
        .args(["--width", "50", "--height", "50"])
        .status()
        .expect("run helix");
    assert!(status.success());
    let bytes = std::fs::read(input.with_extension("png")).expect("png written");
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
}

#[test]
fn binary_migrate_without_action_is_an_error() {
    let scratch = Scratch::new("bin-migrate");
    let input = scratch.path().join("app.html");
    std::fs::write(&input, "<p>hi</p>").unwrap();
    let out = helix().arg("migrate").arg(&input).output().expect("run");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing to do"));
}

#[test]
fn binary_run_of_a_missing_file_reports_an_error() {
    let out = helix()
        .args(["run", "definitely-missing.wasm"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error:"));
}
