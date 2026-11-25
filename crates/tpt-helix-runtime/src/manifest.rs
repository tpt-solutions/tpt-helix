//! Capability manifest inference and capability diff (spec §10.1 and §10.2).
//!
//! A WASM component lists the WIT interfaces it imports. Each Helix interface
//! maps to one grant. wit-bindgen imports only the interfaces a guest calls, so
//! the imported set is the minimal manifest for that guest. Imports that are not
//! Helix interfaces are reported and never granted.

use std::collections::BTreeSet;
use std::fmt;

use wasmparser::{Parser, Payload};

/// The Helix capability interfaces, one grant each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grant {
    Dom,
    Storage,
    Network,
    Media,
}

impl Grant {
    /// The grant name used by `helix run --grant` (`dom`, `storage`, ...).
    pub fn name(self) -> &'static str {
        match self {
            Grant::Dom => "dom",
            Grant::Storage => "storage",
            Grant::Network => "network",
            Grant::Media => "media",
        }
    }

    /// The grant for a WIT import name such as `helix:runtime/dom@0.1.0`.
    /// Returns `None` for anything outside the `helix:runtime` package.
    pub fn from_import(import: &str) -> Option<Grant> {
        let rest = import.strip_prefix("helix:runtime/")?;
        let interface = rest.split('@').next().unwrap_or(rest);
        match interface {
            "dom" => Some(Grant::Dom),
            "storage" => Some(Grant::Storage),
            "network" => Some(Grant::Network),
            "media" => Some(Grant::Media),
            _ => None,
        }
    }
}

/// The grants a component needs, plus the imports that are not Helix interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    pub grants: BTreeSet<Grant>,
    /// Imports outside `helix:runtime`. They are never granted. A component that
    /// needs one of these will not instantiate under the Helix host.
    pub unknown_imports: Vec<String>,
}

impl Manifest {
    /// The grants as a comma-separated list, in the form `helix run --grant` takes.
    pub fn grants_csv(&self) -> String {
        self.grants
            .iter()
            .map(|g| g.name())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// The change in grants between two versions of a component (spec §10.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrantDiff {
    /// Grants the new version needs and the old one did not. These need review.
    pub added: Vec<Grant>,
    pub removed: Vec<Grant>,
}

impl GrantDiff {
    /// Whether the update asks for any grant the old version did not have.
    pub fn requests_new_grants(&self) -> bool {
        !self.added.is_empty()
    }
}

/// Errors reading a component's imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot read component imports: {}", self.0)
    }
}

impl std::error::Error for ManifestError {}

/// Infers the manifest of the WASM component in `bytes` from its top-level
/// imports. Imports of nested components and core modules are not part of the
/// component's own interface, so they are skipped.
pub fn infer(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    let mut manifest = Manifest::default();
    let mut depth: usize = 0;
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| ManifestError(e.to_string()))?;
        match payload {
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(reader) if depth == 0 => {
                for import in reader {
                    let import = import.map_err(|e| ManifestError(e.to_string()))?;
                    let name = import.name.name;
                    match Grant::from_import(name) {
                        Some(grant) => {
                            manifest.grants.insert(grant);
                        }
                        None => manifest.unknown_imports.push(name.to_string()),
                    }
                }
            }
            _ => {}
        }
    }
    Ok(manifest)
}

/// The grants that differ between `old` and `new` (spec §10.2).
pub fn diff(old: &Manifest, new: &Manifest) -> GrantDiff {
    GrantDiff {
        added: new.grants.difference(&old.grants).copied().collect(),
        removed: old.grants.difference(&new.grants).copied().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(grants: &[Grant]) -> Manifest {
        Manifest {
            grants: grants.iter().copied().collect(),
            unknown_imports: vec![],
        }
    }

    #[test]
    fn maps_helix_interface_names_to_grants() {
        assert_eq!(
            Grant::from_import("helix:runtime/dom@0.1.0"),
            Some(Grant::Dom)
        );
        assert_eq!(
            Grant::from_import("helix:runtime/network"),
            Some(Grant::Network)
        );
        assert_eq!(
            Grant::from_import("helix:runtime/storage@0.1.0"),
            Some(Grant::Storage)
        );
        assert_eq!(
            Grant::from_import("helix:runtime/media@0.1.0"),
            Some(Grant::Media)
        );
    }

    #[test]
    fn ignores_interfaces_outside_helix_runtime() {
        assert_eq!(Grant::from_import("wasi:cli/stdout@0.2.0"), None);
        assert_eq!(Grant::from_import("helix:runtime/unknown@0.1.0"), None);
    }

    #[test]
    fn grant_names_match_the_cli_grant_syntax() {
        for grant in [Grant::Dom, Grant::Storage, Grant::Network, Grant::Media] {
            assert_eq!(
                crate::manifest::Grant::from_import(&format!(
                    "helix:runtime/{}@0.1.0",
                    grant.name()
                )),
                Some(grant)
            );
        }
    }

    #[test]
    fn infers_the_imports_of_a_real_guest() {
        // `hello_dom.wasm` is the componentized guest example. It calls dom,
        // storage, and network, so those are the grants it needs.
        let bytes = include_bytes!("../tests/hello_dom.wasm");
        let m = infer(bytes).expect("a valid component");
        assert!(m.grants.contains(&Grant::Dom));
        assert!(m.grants.contains(&Grant::Storage));
        assert!(m.grants.contains(&Grant::Network));
        assert!(m.unknown_imports.is_empty(), "{:?}", m.unknown_imports);
    }

    #[test]
    fn rejects_bytes_that_are_not_wasm() {
        assert!(infer(b"not a component").is_err());
    }

    #[test]
    fn diff_reports_added_and_removed_grants() {
        let old = manifest(&[Grant::Dom, Grant::Storage]);
        let new = manifest(&[Grant::Dom, Grant::Network]);
        let d = diff(&old, &new);
        assert_eq!(d.added, vec![Grant::Network]);
        assert_eq!(d.removed, vec![Grant::Storage]);
        assert!(d.requests_new_grants());
    }

    #[test]
    fn diff_of_a_narrowing_update_requests_nothing_new() {
        let old = manifest(&[Grant::Dom, Grant::Network]);
        let new = manifest(&[Grant::Dom]);
        let d = diff(&old, &new);
        assert!(d.added.is_empty());
        assert_eq!(d.removed, vec![Grant::Network]);
        assert!(!d.requests_new_grants());
    }

    #[test]
    fn grants_csv_is_the_grant_flag_syntax() {
        let m = manifest(&[Grant::Storage, Grant::Dom]);
        assert_eq!(m.grants_csv(), "dom,storage");
        assert_eq!(Manifest::default().grants_csv(), "");
    }
}
