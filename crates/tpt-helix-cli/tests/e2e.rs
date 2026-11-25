//! End-to-end: a migrated app's click reaches its translated handler in the
//! runtime and writes through the storage capability.
//!
//! The pipeline is: migrate an HTML page and its JavaScript, write the generated
//! guest crate, build it for `wasm32-unknown-unknown`, componentize it with
//! `wasm-tools`, then load and run it in the runtime and fire the click.
//!
//! It needs the `wasm32-unknown-unknown` target and `wasm-tools` on PATH, so it is
//! `#[ignore]`d. Run it with:
//!
//! ```sh
//! cargo test -p tpt-helix-cli --test e2e -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use tpt_helix_cli::{migrate_html, write_guest_crate};
use tpt_helix_runtime::capability::{Capability, DomScope, StorageScope};
use tpt_helix_runtime::wasm::{Host, Module, sandboxed_engine};

fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("tpt-helix-e2e")
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

fn run_tool(mut command: Command, what: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|e| format!("could not run {what}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[test]
#[ignore = "needs the wasm32-unknown-unknown target and wasm-tools; run with --ignored"]
fn a_migrated_click_handler_writes_storage_in_the_runtime() {
    let html = r#"<button onclick="save()">Save</button>"#;
    let js = "function save() { localStorage.setItem('note', 'hello'); }";
    let (_migration, guest) = migrate_html("notes", html, Some(js));
    let guest = guest.expect("a page with a button is supported");

    // A workspace shaped like the repository, so the generated `../tpt-helix-wit/wit`
    // path resolves to the real interface definitions.
    let root = scratch_root().join("workspace");
    let _ = std::fs::remove_dir_all(&root);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../tpt-helix-wit/wit"),
        &root.join("crates/tpt-helix-wit/wit"),
    )
    .expect("copy WIT");
    write_guest_crate(&root.join("crates/notes"), &guest).expect("write guest crate");
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/notes\"]\n\n\
         [workspace.package]\nlicense = \"MIT OR Apache-2.0\"\n",
    )
    .expect("write workspace manifest");

    // Build the core module. One target directory keeps the dependencies cached.
    let target_dir = scratch_root().join("target");
    let mut build = Command::new("cargo");
    build
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .args(["-p", "notes"]);
    run_tool(build, "cargo build").expect("the generated guest builds");
    let core = target_dir.join("wasm32-unknown-unknown/release/notes.wasm");

    // Componentize it, as the runtime loads components.
    let component = scratch_root().join("notes.component.wasm");
    let mut wrap = Command::new("wasm-tools");
    wrap.args(["component", "new"])
        .arg(&core)
        .arg("-o")
        .arg(&component);
    run_tool(wrap, "wasm-tools component new").expect("the core module componentizes");

    // Load and run it under the grants the app needs, then fire the click.
    let bytes = std::fs::read(&component).expect("read component");
    let engine = sandboxed_engine().expect("sandboxed engine");
    let module = Module::load(&engine, &bytes).expect("component loads");
    let host = Host::with_grants(
        "notes",
        [
            Capability::Dom {
                scope: DomScope::Full,
            },
            Capability::Storage {
                scope: StorageScope::Global,
            },
        ],
    );
    let mut instance = module.instantiate(&engine, host).expect("instantiates");
    instance.run().expect("run registers the click handler");

    // The button is the element that run registered the click handler on.
    let button = (0..64u64)
        .find(|&id| !instance.host().state.handler_ids(id, "click").is_empty())
        .expect("run registers a click handler on some element");
    let ran = instance.fire_event(button, "click").expect("dispatch");
    assert_eq!(ran, 1, "one click handler should run");
    assert_eq!(
        instance.host().stored("note"),
        Some(b"hello".to_vec()),
        "the translated handler should write storage"
    );
}
