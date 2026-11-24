//! `helix` developer CLI (spec §9 developer tooling).
//!
//! Subcommands are implemented here as plain functions so they can be tested
//! without spawning the binary; `main.rs` only parses arguments and prints.
//!
//! * [`render_html`] — headless PNG render of an HTML document (`helix render`).
//! * [`run_module`] — instantiate a WASM guest under an explicit capability set
//!   and call its `run` export (`helix run`).
//! * [`migrate_html`] / [`migration_report`] / [`write_guest_crate`] — the S1–S5
//!   migration pipeline and its report (`helix migrate`).
//! * [`build_guest`] — build a guest crate for `wasm32-unknown-unknown`
//!   (`helix build`).
//! * [`new_project`] — scaffold a static starter app (`helix new`).
//! * [`doctor_checks`] — toolchain readiness report (`helix doctor`).

use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;

use tpt_helix_migrate::deploy::TargetPlatform;
use tpt_helix_migrate::eve::{self, Fit, OrchestratedMigration, PlanRequest};
use tpt_helix_migrate::generate_guest;
use tpt_helix_migrate::spark::GuestCrate;
use tpt_helix_runtime::capability::{Capability, DomScope, HostPattern, StorageScope};
use tpt_helix_runtime::css::parse_stylesheet;
use tpt_helix_runtime::display_list::build_display_list;
use tpt_helix_runtime::html::{inline_styles, parse_html};
use tpt_helix_runtime::layout::{build_layout_tree, compute};
use tpt_helix_runtime::software_raster::{encode_png, rasterize_display_list};
use tpt_helix_runtime::wasm::{Host, Module, sandboxed_engine};

/// Largest viewport edge `render` accepts, in pixels.
pub const MAX_VIEWPORT: u32 = 8192;

/// Error reported by any subcommand. Rendered as `error: <message>` by the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError(e.to_string())
    }
}

fn err(msg: impl Into<String>) -> CliError {
    CliError(msg.into())
}

// --- render ----------------------------------------------------------------

/// Renders `html` (plus its inline `<style>` blocks and `extra_css`) to a PNG
/// at `width` x `height`, using the same static pipeline the GPU presenter uses.
pub fn render_html(
    html: &str,
    extra_css: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CliError> {
    if width == 0 || height == 0 || width > MAX_VIEWPORT || height > MAX_VIEWPORT {
        return Err(err(format!(
            "viewport must be between 1 and {MAX_VIEWPORT} pixels per side"
        )));
    }
    let dom = parse_html(html);
    let css = format!("{}\n{}", inline_styles(&dom), extra_css);
    let rules = parse_stylesheet(&css);
    let mut layout =
        build_layout_tree(&dom, &rules).map_err(|e| err(format!("layout failed: {e:?}")))?;
    compute(&mut layout, width as f32, height as f32)
        .map_err(|e| err(format!("layout failed: {e:?}")))?;
    let items = build_display_list(&layout.tree, layout.root, &rules);
    let image = rasterize_display_list(&items, width, height);
    Ok(encode_png(&image))
}

// --- run -------------------------------------------------------------------

/// A capability the `run` subcommand may grant to a guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Full DOM access.
    Dom,
    /// Read/write any storage key.
    Storage,
    /// Network access to any host.
    Network,
    /// Media playback up to 1920x1080.
    Media,
}

/// Parses a comma-separated grant list such as `dom,storage`. Empty input means
/// no grants (the deny-by-default posture).
pub fn parse_grants(spec: &str) -> Result<Vec<Grant>, CliError> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        out.push(match part {
            "dom" => Grant::Dom,
            "storage" => Grant::Storage,
            "network" => Grant::Network,
            "media" => Grant::Media,
            other => {
                return Err(err(format!(
                    "unknown capability `{other}` (expected dom, storage, network, media)"
                )));
            }
        });
    }
    Ok(out)
}

fn grant_capabilities(grants: &[Grant]) -> Vec<Capability> {
    grants
        .iter()
        .map(|g| match g {
            Grant::Dom => Capability::Dom {
                scope: DomScope::Full,
            },
            Grant::Storage => Capability::Storage {
                scope: StorageScope::Global,
            },
            Grant::Network => Capability::Network {
                hosts: vec![HostPattern::Any],
            },
            Grant::Media => Capability::Media {
                max_resolution: Some((1920, 1080)),
            },
        })
        .collect()
}

/// Instantiates the WASM component `bytes` as app `app` and calls its `run`
/// export. Only `grants` are authorized; everything else is denied. With
/// `unrestricted`, the capability broker is bypassed entirely (trusted local
/// development only).
pub fn run_module(
    bytes: &[u8],
    app: &str,
    grants: &[Grant],
    unrestricted: bool,
) -> Result<(), CliError> {
    let engine = sandboxed_engine().map_err(|e| err(format!("engine: {e}")))?;
    let module = Module::load(&engine, bytes).map_err(|e| err(format!("invalid module: {e}")))?;
    let host = if unrestricted {
        Host::unrestricted()
    } else {
        Host::with_grants(app, grant_capabilities(grants))
    };
    let mut instance = module
        .instantiate(&engine, host)
        .map_err(|e| err(format!("instantiation failed: {e}")))?;
    instance
        .run()
        .map_err(|e| err(format!("guest trapped: {e}")))
}

// --- migrate ---------------------------------------------------------------

/// Runs the S1–S5 pipeline for one HTML document (and optional JS source).
/// Returns Eve's orchestration always, and the generated guest crate only when a
/// supported pattern matched. An unsupported document still gets a report.
pub fn migrate_html(
    name: &str,
    html: &str,
    js: Option<&str>,
) -> (OrchestratedMigration, Option<GuestCrate>) {
    let req = PlanRequest {
        app_name: name.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        html: html.to_string(),
        js: js.map(str::to_string),
        targets: vec![TargetPlatform::Wasm32],
    };
    let migration = eve::orchestrate(&req);
    let guest = if migration.plan.confidence == Fit::None {
        None
    } else {
        Some(generate_guest(&req, &migration))
    };
    (migration, guest)
}

/// Human-readable migration report: pattern, per-stage plan, equivalence, and
/// explicit warnings for anything that needs manual attention.
pub fn migration_report(m: &OrchestratedMigration) -> String {
    let mut out = String::new();
    out.push_str(&format!("app: {}\n", m.plan.app_name));
    out.push_str(&format!(
        "pattern: {:?} (confidence: {:?})\n",
        m.plan.detected, m.plan.confidence
    ));
    out.push_str("stages:\n");
    for stage in &m.plan.stages {
        let state = if stage.enabled { "run" } else { "skip" };
        out.push_str(&format!(
            "  S{} {:<10} {:<4} {}\n",
            stage.stage, stage.name, state, stage.note
        ));
    }
    out.push_str(&format!(
        "equivalence (S3): {}\n",
        if m.equivalence_ok { "ok" } else { "FAILED" }
    ));
    out.push_str(&format!("suggestions (S4): {}\n", m.suggestions.len()));
    out.push_str(&format!(
        "symbols (S1): {} imports, {} exports, {} functions\n",
        m.surface.imports.len(),
        m.surface.exports.len(),
        m.surface.functions.len()
    ));
    for warning in migration_warnings(m) {
        out.push_str(&format!("warning: {warning}\n"));
    }
    out
}

/// Conditions that need a human before the migrated app is deployed.
pub fn migration_warnings(m: &OrchestratedMigration) -> Vec<String> {
    let mut warnings = Vec::new();
    match m.plan.confidence {
        Fit::Strong => {}
        Fit::Partial => warnings.push(
            "partial pattern match: review the generated guest before deploying".to_string(),
        ),
        Fit::None => warnings.push(
            "no supported pattern matched: this app needs manual migration".to_string(),
        ),
    }
    if !m.equivalence_ok {
        warnings.push("equivalence check failed: do not deploy this migration".to_string());
    }
    warnings
}

/// Writes a generated guest crate under `dir` (`Cargo.toml`, `src/lib.rs`, and
/// the WIT world file). Refuses to write into a non-empty existing directory.
pub fn write_guest_crate(dir: &Path, guest: &GuestCrate) -> Result<(), CliError> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        return Err(err(format!(
            "{} exists and is not empty; choose another --out directory",
            dir.display()
        )));
    }
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join("Cargo.toml"), &guest.cargo_toml)?;
    fs::write(dir.join("src").join("lib.rs"), &guest.lib_rs)?;
    fs::write(dir.join("helix-guest.wit"), &guest.world_wit)?;
    Ok(())
}

// --- build -----------------------------------------------------------------

/// Builds the guest crate in `dir` for `wasm32-unknown-unknown` in release mode.
pub fn build_guest(dir: &Path) -> Result<(), CliError> {
    let status = Command::new("cargo")
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(dir)
        .status()
        .map_err(|e| err(format!("could not run cargo: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(err("cargo build failed (run `helix doctor` to check the toolchain)"))
    }
}

// --- new -------------------------------------------------------------------

const STARTER_HTML: &str = r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <title>Hello Helix</title>
    <style>
      body { margin: 0; }
      .card { width: 320px; height: 120px; background-color: #3366ff; }
    </style>
  </head>
  <body>
    <div class="card"></div>
  </body>
</html>
"#;

/// Scaffolds a static starter app in `dir`. Fails if `dir` already exists.
pub fn new_project(dir: &Path) -> Result<(), CliError> {
    if dir.exists() {
        return Err(err(format!("{} already exists", dir.display())));
    }
    fs::create_dir_all(dir)?;
    fs::write(dir.join("index.html"), STARTER_HTML)?;
    Ok(())
}

// --- doctor ----------------------------------------------------------------

/// One readiness check for `helix doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    /// Whether a failure here blocks core workflows (`build`, `doctor` exit).
    pub required: bool,
    pub ok: bool,
    /// Version string or reason the check failed.
    pub detail: String,
    /// What to do about a failure.
    pub hint: &'static str,
}

/// Runs `program args...` and returns its first stdout line when it succeeds.
fn probe(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(|l| l.trim().to_string())
}

/// Checks the toolchain needed by the helix workflows.
pub fn doctor_checks() -> Vec<Check> {
    let mut checks = Vec::new();

    let cargo = probe("cargo", &["--version"]);
    checks.push(Check {
        name: "cargo",
        required: true,
        ok: cargo.is_some(),
        detail: cargo.unwrap_or_else(|| "not found on PATH".into()),
        hint: "install Rust from https://rustup.rs",
    });

    let rustc = probe("rustc", &["--version"]);
    checks.push(Check {
        name: "rustc",
        required: true,
        ok: rustc.is_some(),
        detail: rustc.unwrap_or_else(|| "not found on PATH".into()),
        hint: "install Rust from https://rustup.rs",
    });

    let wasm_target = match probe("rustup", &["target", "list", "--installed"]) {
        Some(_) => {
            let listing = Command::new("rustup")
                .args(["target", "list", "--installed"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let installed = listing.lines().any(|l| l.trim() == "wasm32-unknown-unknown");
            (
                installed,
                if installed {
                    "wasm32-unknown-unknown installed".to_string()
                } else {
                    "wasm32-unknown-unknown not installed".to_string()
                },
            )
        }
        None => (false, "rustup not found on PATH".to_string()),
    };
    checks.push(Check {
        name: "wasm32 target",
        required: true,
        ok: wasm_target.0,
        detail: wasm_target.1,
        hint: "run `rustup target add wasm32-unknown-unknown`",
    });

    let wasm_tools = probe("wasm-tools", &["--version"]);
    checks.push(Check {
        name: "wasm-tools",
        required: false,
        ok: wasm_tools.is_some(),
        detail: wasm_tools.unwrap_or_else(|| "not found on PATH".into()),
        hint: "run `cargo install wasm-tools` to inspect and validate components",
    });

    if cfg!(windows) {
        let patch = probe("patch", &["--version"]);
        checks.push(Check {
            name: "patch",
            required: true,
            ok: patch.is_some(),
            detail: patch.unwrap_or_else(|| "not found on PATH".into()),
            hint: "add Git for Windows' usr/bin to PATH (rquickjs's build script needs `patch`)",
        });
    }

    checks
}

/// True when every required check passed.
pub fn all_required_ok(checks: &[Check]) -> bool {
    checks.iter().all(|c| !c.required || c.ok)
}

/// The starter HTML used by `helix new` (exposed for tests and docs).
pub fn starter_html() -> &'static str {
    STARTER_HTML
}
