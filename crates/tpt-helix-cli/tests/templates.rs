//! Tests for `helix new --template`: the scaffolded guest crates have the right
//! files, substituted names, and grant manifests the CLI accepts. The ignored test
//! also compiles each template for wasm32.

use std::path::{Path, PathBuf};
use std::process::Command;

use tpt_helix_cli::{GuestTemplate, new_guest_project, parse_grants};

const TEMPLATES: &[(&str, GuestTemplate, &str)] = &[
    ("basic", GuestTemplate::Basic, "dom"),
    ("network", GuestTemplate::Network, "dom,network"),
    ("storage", GuestTemplate::Storage, "dom,storage"),
];

/// A fresh path under the OS temp dir for one scaffolded project.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("helix-template-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn every_template_scaffolds_a_complete_guest_crate() {
    for (name, template, _) in TEMPLATES {
        let dir = scratch(name).join("my-app");
        new_guest_project(&dir, *template).expect("scaffold");

        for file in ["Cargo.toml", "src/lib.rs", "capabilities", "wit/helix.wit"] {
            assert!(dir.join(file).is_file(), "{name}: missing {file}");
        }
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(
            cargo.contains("name = \"my-app\""),
            "{name}: name not substituted"
        );
        let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert!(
            !lib.contains("{{name}}"),
            "{name}: placeholder left in lib.rs"
        );
        assert!(
            lib.contains("my-app"),
            "{name}: app name missing from lib.rs"
        );

        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}

#[test]
fn every_template_capability_manifest_is_a_valid_grant_list() {
    for (name, template, expected) in TEMPLATES {
        let dir = scratch(&format!("{name}-grants")).join("app");
        new_guest_project(&dir, *template).expect("scaffold");
        let manifest = std::fs::read_to_string(dir.join("capabilities")).unwrap();
        assert_eq!(manifest.trim(), *expected, "{name}: unexpected grant list");
        parse_grants(manifest.trim()).expect("the manifest must parse as --grant");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}

#[test]
fn scaffolded_wit_is_the_canonical_interface_file() {
    let dir = scratch("wit").join("app");
    new_guest_project(&dir, GuestTemplate::Basic).expect("scaffold");
    let wit = std::fs::read_to_string(dir.join("wit/helix.wit")).unwrap();
    assert_eq!(wit, tpt_helix_wit::HELIX_WIT);
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn unknown_template_is_rejected() {
    assert!(GuestTemplate::parse("nope").is_err());
    assert_eq!(
        GuestTemplate::parse("network").unwrap(),
        GuestTemplate::Network
    );
}

#[test]
fn existing_directory_is_never_overwritten() {
    let dir = scratch("exists");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(new_guest_project(&dir, GuestTemplate::Basic).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs the wasm32-unknown-unknown target; run with --ignored"]
fn every_template_compiles_for_wasm32() {
    let mut failures = Vec::new();
    for (name, template, _) in TEMPLATES {
        let dir = scratch(&format!("{name}-check")).join("app");
        new_guest_project(&dir, *template).expect("scaffold");
        if let Err(detail) = check_wasm(&dir) {
            failures.push(format!("{name}:\n{detail}"));
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
    assert!(
        failures.is_empty(),
        "templates do not compile:\n\n{}",
        failures.join("\n\n")
    );
}

fn check_wasm(dir: &Path) -> Result<(), String> {
    let output = Command::new("cargo")
        .args(["check", "--target", "wasm32-unknown-unknown", "--quiet"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
