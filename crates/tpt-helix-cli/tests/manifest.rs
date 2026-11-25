//! Tests for `helix manifest` (spec §10.1 and §10.2) against the componentized
//! guest example, which calls dom, storage, and network.

use std::path::Path;

use tpt_helix_cli::{diff_report, manifest_report, read_manifest};
use tpt_helix_runtime::manifest::{self, Grant};

fn guest() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tpt-helix-runtime/tests/hello_dom.wasm"
    ))
}

#[test]
fn report_lists_the_inferred_grants() {
    let m = read_manifest(guest()).expect("read the guest's imports");
    let report = manifest_report(&m);
    assert_eq!(report, "grants: dom,storage,network\n");
}

#[test]
fn a_guest_is_granted_only_the_interfaces_it_imports() {
    let m = read_manifest(guest()).unwrap();
    assert!(
        !m.grants.contains(&Grant::Media),
        "the guest never calls media"
    );
}

#[test]
fn comparing_a_component_with_itself_requests_nothing_new() {
    let m = read_manifest(guest()).unwrap();
    let d = manifest::diff(&m, &m);
    assert!(!d.requests_new_grants());
    assert!(diff_report(&d).contains("added: (none)"));
}

#[test]
fn an_update_that_adds_a_grant_is_flagged_for_review() {
    let old = manifest::Manifest {
        grants: [Grant::Dom].into_iter().collect(),
        unknown_imports: vec![],
    };
    let new = read_manifest(guest()).unwrap();
    let report = diff_report(&manifest::diff(&old, &new));
    assert!(report.contains("added: storage,network"), "{report}");
    assert!(report.contains("review:"), "{report}");
}
