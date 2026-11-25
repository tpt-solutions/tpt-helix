//! Compile-checks the guest crates the migration pipeline generates.
//!
//! Each supported pattern's fixture is migrated, and the generated crate is
//! written into a temporary workspace shaped like the repository (so the
//! `../tpt-helix-wit/wit` path inside the generated code resolves). Then
//! `cargo check --target wasm32-unknown-unknown` runs on it.
//!
//! The test is slow and needs the wasm32 target, so it is `#[ignore]`d. Run it
//! with:
//!
//! ```sh
//! cargo test -p tpt-helix-migrate --test guest_build -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use tpt_helix_migrate::coverage::Pattern;
use tpt_helix_migrate::deploy::TargetPlatform;
use tpt_helix_migrate::eve::PlanRequest;
use tpt_helix_migrate::migrate;
use tpt_helix_migrate::spark::GuestCrate;

fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("tpt-helix-guest-check")
}

fn repo_wit_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tpt-helix-wit/wit")
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Writes `guest` into a repo-shaped scratch workspace and runs `cargo check`
/// for wasm32. Returns the tail of the compiler output on failure.
fn check_generated(name: &str, guest: &GuestCrate) -> Result<(), String> {
    let root = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&root);
    let crate_dir = root.join("crates").join(name);
    copy_dir(&repo_wit_dir(), &root.join("crates/tpt-helix-wit/wit"))
        .map_err(|e| format!("copying WIT: {e}"))?;
    std::fs::create_dir_all(crate_dir.join("src")).map_err(|e| e.to_string())?;
    // The generated manifest inherits `license` from its workspace, so the
    // scratch root is a workspace that defines it, with the guest as a member.
    let root_manifest = format!(
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/{name}\"]\n\n[workspace.package]\nlicense = \"MIT OR Apache-2.0\"\n"
    );
    std::fs::write(root.join("Cargo.toml"), root_manifest).map_err(|e| e.to_string())?;
    std::fs::write(crate_dir.join("Cargo.toml"), &guest.cargo_toml).map_err(|e| e.to_string())?;
    std::fs::write(crate_dir.join("src/lib.rs"), &guest.lib_rs).map_err(|e| e.to_string())?;

    // One target directory for every fixture, so the dependencies compile once.
    let output = Command::new("cargo")
        .args([
            "check",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(scratch_root().join("target"))
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("-p")
        .arg(name)
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(40).collect();
        Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
    }
}

fn migrate_fixture(pattern: Pattern) -> GuestCrate {
    let name = format!("{pattern:?}").to_lowercase();
    let (_, guest) = migrate(&PlanRequest {
        app_name: name,
        version: "0.1.0".into(),
        html: pattern.sample().to_string(),
        js: None,
        targets: vec![TargetPlatform::Wasm32],
    })
    .expect("every supported fixture migrates");
    guest
}

#[test]
#[ignore = "needs the wasm32-unknown-unknown target and compiles each guest; run with --ignored"]
fn every_supported_pattern_generates_a_guest_that_compiles() {
    let mut results = Vec::new();
    for pattern in Pattern::ALL {
        let name = format!("{pattern:?}").to_lowercase();
        let guest = migrate_fixture(*pattern);
        results.push((name.clone(), check_generated(&name, &guest)));
    }

    println!("\nguest compile check (cargo check --target wasm32-unknown-unknown):");
    let mut failures = Vec::new();
    for (name, result) in &results {
        match result {
            Ok(()) => println!("  ok      {name}"),
            Err(detail) => {
                println!("  FAILED  {name}");
                failures.push(format!("{name}:\n{detail}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} generated guest(s) do not compile:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
#[ignore = "needs the wasm32-unknown-unknown target and compiles the guest; run with --ignored"]
fn a_click_handler_calls_its_translated_function_and_compiles() {
    let html = r#"<html><body><h1>Counter</h1><button onclick="greet()">Say hi</button></body></html>"#;
    let js = "function greet() { console.log('hi'); }";
    let (migration, guest) = migrate(&PlanRequest {
        app_name: "clicker".into(),
        version: "0.1.0".into(),
        html: html.into(),
        js: Some(js.into()),
        targets: vec![TargetPlatform::Wasm32],
    })
    .expect("a page with a button is supported");

    assert!(
        migration.warnings().is_empty(),
        "unexpected warnings: {:?}",
        migration.warnings()
    );
    assert!(
        guest.lib_rs.contains("0 => greet(),"),
        "dispatch does not call greet:\n{}",
        guest.lib_rs
    );
    if let Err(detail) = check_generated("clicker", &guest) {
        panic!("the generated guest does not compile:\n{detail}");
    }
}
