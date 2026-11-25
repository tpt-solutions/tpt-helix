//! Keeps `spec.txt` §5.2 in sync with the authored WIT.
//!
//! `crates/tpt-helix-wit/wit/helix.wit` is the source of truth. The spec carries a
//! verbatim copy between `[BEGIN helix.wit]` and `[END helix.wit]`. Any drift
//! fails this test, so the spec cannot describe an interface the bindings do not
//! implement.

use std::path::PathBuf;

fn repo_file(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").trim().to_string()
}

fn spec_block() -> String {
    let spec = std::fs::read_to_string(repo_file("../../spec.txt")).expect("read spec.txt");
    let spec = normalize(&spec);
    let begin = spec
        .find("[BEGIN helix.wit]")
        .expect("spec.txt is missing the [BEGIN helix.wit] marker");
    let end = spec
        .find("[END helix.wit]")
        .expect("spec.txt is missing the [END helix.wit] marker");
    assert!(begin < end, "[END helix.wit] must follow [BEGIN helix.wit]");
    normalize(&spec[begin + "[BEGIN helix.wit]".len()..end])
}

#[test]
fn spec_wit_block_matches_helix_wit() {
    let wit = std::fs::read_to_string(repo_file("wit/helix.wit")).expect("read helix.wit");
    assert_eq!(
        spec_block(),
        normalize(&wit),
        "spec.txt §5.2 has drifted from wit/helix.wit; update both together"
    );
}

#[test]
fn spec_declares_the_worlds_the_bindings_generate() {
    let block = spec_block();
    // `helix-guest` is the world `lib.rs` generates the guest bindings from, and
    // `helix-host` the world it generates the host traits from.
    assert!(block.contains("world helix-host {"));
    assert!(block.contains("world helix-guest {"));
    assert!(
        !block.contains("world helix {"),
        "the guest world is named helix-guest"
    );
}
