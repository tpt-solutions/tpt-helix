//! Fuzz/property test for the WIT interface boundary against malformed guest
//! input (per the TODO "fuzz WIT interface boundary parsing (`network`/
//! `storage`/`dom`/`media`) against malformed input").
//!
//! The "guest" side of the boundary is whatever bytes/strings a module chooses
//! to send. This hammers the host-side implementations — both the in-memory
//! `RuntimeStub` (legacy/QuickJS path) and the `wasmtime` `Host` (WASM path) —
//! with adversarial inputs (unprintable bytes, control chars, very long
//! strings, empty keys, duplicate ids, oversized payloads) and asserts the
//! boundary never panics and stays internally consistent.

use helix_runtime::stub::RuntimeStub;
use helix_runtime::wasm::Host;
use helix_wit::host::exports::helix::runtime::{
    dom as wit_dom, media as wit_media, network as wit_net,
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
    /// A string drawn from a hostile alphabet: printable ASCII plus control
    /// characters, NUL, and a few multi-byte UTF-8 sequences.
    fn string(&mut self, max_len: usize) -> String {
        let alphabet: &[char] = &[
            'a', 'Z', '0', ' ', '\n', '\t', '\r', '\0', '\u{1f}', '\u{7f}', 'é', '本', '🦀',
        ];
        let n = self.below(max_len + 1);
        (0..n)
            .map(|_| alphabet[self.below(alphabet.len())])
            .collect()
    }
    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let n = self.below(max + 1);
        (0..n).map(|_| self.below(256) as u8).collect()
    }
}

#[test]
fn stub_boundary_survives_malformed_input() {
    let mut rng = Rng(0xc0ffee);
    let stub = RuntimeStub::new();

    // Create a pool of elements with hostile tags, then operate on them with
    // hostile text/attributes. The boundary must not panic.
    let mut ids = Vec::new();
    for _ in 0..200 {
        let id = RuntimeStub::create_element(rng.string(16));
        ids.push(id);
        // Even a nonsense id (u64::MAX) must be safe to pass.
        let target = if rng.below(3) == 0 {
            wit_dom::ElementId { id: u64::MAX }
        } else {
            id
        };
        RuntimeStub::set_text(target, rng.string(64));
        RuntimeStub::set_attribute(target, rng.string(32), rng.string(64));
        RuntimeStub::on_click(target, rng.next());
        if ids.len() >= 2 {
            RuntimeStub::append_child(ids[rng.below(ids.len())], ids[rng.below(ids.len())]);
        }
    }

    // Storage with empty/oversized/control-char keys and payloads.
    for _ in 0..200 {
        let key = rng.string(48);
        let val = rng.bytes(1024);
        // set must either succeed or error gracefully — never panic.
        let _ = RuntimeStub::set(key.clone(), val.clone());
        let _ = RuntimeStub::get(key.clone());
        let _ = RuntimeStub::delete(key);
    }

    // Network boundary: malformed URLs, empty method, huge bodies.
    for _ in 0..200 {
        let url = rng.string(128);
        stub.register_fetch(
            &url,
            wit_net::Response {
                status: rng.below(600) as u16,
                headers: vec![],
                body: rng.bytes(2048),
            },
        );
        let _ = RuntimeStub::fetch(wit_net::Request {
            method: rng.string(8),
            url,
            headers: vec![],
            body: if rng.below(2) == 0 {
                None
            } else {
                Some(rng.bytes(256))
            },
        });
    }

    // Media boundary: extreme resolutions / bitrates must not panic.
    for _ in 0..=100 {
        let cfg = wit_media::VideoConfig {
            codec: rng.string(16),
            width: rng.below(100_000) as u32,
            height: rng.below(100_000) as u32,
            bitrate: rng.next() as u32,
        };
        if let Ok(h) = RuntimeStub::create_player(cfg) {
            RuntimeStub::play(h);
            RuntimeStub::seek(h, rng.next());
            RuntimeStub::pause(h);
        }
    }
}

#[test]
fn wasm_host_boundary_survives_malformed_input() {
    use helix_runtime::wasm::bindings::helix::runtime::{
        dom, dom::Host as DomHost, media, media::Host as MediaHost, network,
        network::Host as NetworkHost, storage::Host as StorageHost,
    };
    let mut rng = Rng(0xbeefcafe);
    let mut host = Host::new();

    let mut ids = Vec::new();
    for _ in 0..200 {
        let el = host.create_element(rng.string(16));
        ids.push(el);
        let target = if rng.below(3) == 0 {
            dom::ElementId { id: u64::MAX }
        } else {
            el
        };
        host.set_text(target, rng.string(64));
        host.set_attribute(target, rng.string(32), rng.string(64));
        host.on_click(target, rng.next());
        if ids.len() >= 2 {
            host.append_child(ids[rng.below(ids.len())], ids[rng.below(ids.len())]);
        }
    }

    for _ in 0..200 {
        let key = rng.string(48);
        let val = rng.bytes(1024);
        let _ = host.set(key.clone(), val);
        let _ = host.get(key.clone());
        let _ = host.delete(key);
    }

    for _ in 0..200 {
        let url = rng.string(128);
        let _ = host.fetch(network::Request {
            method: rng.string(8),
            url,
            headers: vec![],
            body: if rng.below(2) == 0 {
                None
            } else {
                Some(rng.bytes(256))
            },
        });
    }

    for _ in 0..100 {
        let cfg = media::VideoConfig {
            codec: rng.string(16),
            width: rng.below(100_000) as u32,
            height: rng.below(100_000) as u32,
            bitrate: rng.next() as u32,
        };
        if let Ok(h) = host.create_player(cfg) {
            host.play(h);
            host.seek(h, rng.next());
            host.pause(h);
        }
    }
}

/// A consistency property for the `dom` boundary: cycles (a→b→a) and duplicate
/// appends must be safe (no panic, no double-free), and querying remains valid.
#[test]
fn dom_boundary_append_cycles_are_safe() {
    use helix_runtime::wasm::bindings::helix::runtime::dom::Host as DomHost;
    let mut host = Host::new();
    let a = host.create_element("div".into());
    let b = host.create_element("div".into());
    host.append_child(a, b);
    host.append_child(b, a); // cycle back — must not panic or hang
    assert!(host.element(a).is_some());
    assert!(host.element(b).is_some());

    // Appending the same child twice under the same parent is safe.
    host.append_child(a, b);
    assert_eq!(
        host.element(a).unwrap().children.last().map(|e| e.id),
        Some(b.id)
    );
}
