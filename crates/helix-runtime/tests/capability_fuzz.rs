//! Fuzz / property test for the Capability Broker's grant/revoke/delegate
//! state machine (per the TODO "fuzz the capability broker's grant/revoke/
//! delegate state machine").
//!
//! Rather than depend on `cargo-fuzz` (which needs nightly + a fuzz target),
//! this drives the broker with a long deterministic pseudo-random sequence of
//! operations and checks a maintained model of the broker's semantics against
//! the real `check`/`revoke`/`delegate` results. A mismatch is a real bug in
//! either the broker or the model.

use helix_runtime::capability::{
    Capability, CapabilityBroker, CapabilityToken, GrantId, HostPattern,
};

/// Tiny deterministic PRNG (xorshift64*) so runs are reproducible.
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
    fn pick<T: Clone>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())].clone()
    }
}

const HOSTS: &[&str] = &[
    "a.example.com",
    "b.example.com",
    "c.example.com",
    "d.example.com",
];

/// Mirror of a live grant in the broker.
struct Model {
    id: GrantId,
    token: CapabilityToken,
    app: String,
    hosts: Vec<String>, // exact hosts this grant authorizes
    revoked: bool,
    parent: Option<usize>, // index into `models`
}

fn net(hosts: &[&str]) -> Capability {
    Capability::Network {
        hosts: hosts
            .iter()
            .map(|h| HostPattern::Exact(h.to_string()))
            .collect(),
    }
}

fn children_of(models: &[Model], idx: usize) -> Vec<usize> {
    models
        .iter()
        .enumerate()
        .filter(|(i, m)| *i != idx && m.parent == Some(idx))
        .map(|(i, _)| i)
        .collect()
}

/// Mark `idx` (and transitively all delegated descendants) revoked in the
/// model, mirroring the broker's cascade on `revoke` / `revoke_app`.
fn mark_revoked(models: &mut [Model], idx: usize) {
    let mut stack = vec![idx];
    while let Some(i) = stack.pop() {
        models[i].revoked = true;
        let children = children_of(models, i);
        for c in children {
            if !models[c].revoked {
                stack.push(c);
            }
        }
    }
}

#[test]
fn randomized_grant_revoke_delegate_sequence_maintains_invariants() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut broker = CapabilityBroker::new();
    let mut models: Vec<Model> = Vec::new();
    let apps: Vec<String> = (0..4).map(|i| format!("app{i}")).collect();

    for step in 0..4000 {
        match rng.below(5) {
            // grant a fresh capability to a random app
            0 => {
                let app = rng.pick(&apps);
                let n = 1 + rng.below(HOSTS.len());
                let mut hosts: Vec<&str> = HOSTS[..n].to_vec();
                // shuffle a random subset deterministically
                for _ in 0..n {
                    let a = rng.below(hosts.len());
                    let b = rng.below(hosts.len());
                    hosts.swap(a, b);
                }
                let hosts: Vec<String> = hosts.iter().map(|s| s.to_string()).collect();
                let id = broker.grant_count() as u64 + 1; // monotonic estimate
                let token = broker.grant(
                    app.clone(),
                    net(&hosts.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
                );
                // Recover the real grant id via resolve.
                let gid = broker.resolve(token).expect("token resolves");
                assert_eq!(gid.0, id, "grant ids must be issued monotonically");
                models.push(Model {
                    id: gid,
                    token,
                    app: app.clone(),
                    hosts,
                    revoked: false,
                    parent: None,
                });
            }
            // delegate a narrower subset from a live parent
            1 => {
                let live: Vec<usize> = models
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| !m.revoked)
                    .map(|(i, _)| i)
                    .collect();
                if live.is_empty() {
                    continue;
                }
                let pi = rng.pick(&live);
                let parent = &models[pi];
                if parent.hosts.is_empty() {
                    continue;
                }
                // child hosts must be a subset of the parent's hosts
                let child: Vec<String> = parent
                    .hosts
                    .iter()
                    .filter(|_| rng.below(2) == 0)
                    .cloned()
                    .collect::<Vec<_>>();
                if child.is_empty() {
                    continue;
                }
                let child_cos: Vec<&str> = child.iter().map(|s| s.as_str()).collect();
                let child_app = rng.pick(&apps);
                let parent_cap = net(&parent.hosts.iter().map(|s| s.as_str()).collect::<Vec<_>>());
                let _pre = broker.check(parent.token, &parent_cap);
                match broker.delegate(parent.token, child_app.clone(), net(&child_cos)) {
                    Ok(token) => {
                        let gid = broker.resolve(token).expect("delegated token resolves");
                        models.push(Model {
                            id: gid,
                            token,
                            app: child_app.clone(),
                            hosts: child,
                            revoked: false,
                            parent: Some(pi),
                        });
                    }
                    Err(e) => panic!("delegate within parent scope must succeed: {e:?}"),
                }
            }
            // check a random token against a random host
            2 => {
                if models.is_empty() {
                    continue;
                }
                let mi = rng.below(models.len());
                let m = &models[mi];
                let host = rng.pick(HOSTS).to_string();
                let res = broker.check(m.token, &net(&[host.as_str()]));
                let expected_ok = !m.revoked && m.hosts.iter().any(|h| h == &host);
                assert_eq!(
                    res.is_ok(),
                    expected_ok,
                    "step {step}: token check mismatch for host {host} (revoked={}, hosts={:?})",
                    m.revoked,
                    m.hosts
                );
                if let Ok(gid) = res {
                    assert_eq!(gid, m.id, "check must resolve to the token's own grant");
                }
            }
            // revoke a grant (cascades to delegations)
            3 => {
                if models.is_empty() {
                    continue;
                }
                let mi = rng.below(models.len());
                if models[mi].revoked {
                    continue;
                }
                assert!(broker.revoke(models[mi].id));
                mark_revoked(&mut models, mi);
            }
            // revoke every grant of a random app (broker cascades to delegated
            // children regardless of their own app, so the model must too).
            4 => {
                let app = rng.pick(&apps);
                broker.revoke_app(&app);
                for i in 0..models.len() {
                    if models[i].app == app {
                        mark_revoked(&mut models, i);
                    }
                }
                assert!(
                    broker.list_grants(&app).is_empty(),
                    "revoke_app must leave no live grants for {app}"
                );
            }
            _ => unreachable!(),
        }
    }

    // Final global invariant: every revoked model grant reports Revoked on check,
    // every live grant still authorizes its own hosts.
    for m in &models {
        if m.revoked {
            assert!(
                matches!(
                    broker.check(
                        m.token,
                        &net(&m.hosts.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                    ),
                    Err(helix_runtime::capability::CapabilityError::Revoked { .. })
                ),
                "revoked grant must report Revoked"
            );
        } else {
            assert!(
                broker
                    .check(
                        m.token,
                        &net(&m.hosts.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                    )
                    .is_ok(),
                "live grant must still authorize its own hosts"
            );
        }
    }
}

/// Delegating a child, then revoking the *parent*, must revoke the child
/// (cascade) — a targeted property the random fuzz also exercises but which
/// deserves an explicit, readable assertion.
#[test]
fn revoking_parent_revokes_delegated_child() {
    let mut broker = CapabilityBroker::new();
    let parent = broker.grant(
        "parent".into(),
        Capability::Network {
            hosts: vec![HostPattern::Exact("a.example.com".into())],
        },
    );
    let child = broker
        .delegate(
            parent,
            "child".into(),
            Capability::Network {
                hosts: vec![HostPattern::Exact("a.example.com".into())],
            },
        )
        .unwrap();

    assert!(broker.check(child, &net(&["a.example.com"])).is_ok());

    let pid = broker.resolve(parent).unwrap();
    broker.revoke(pid);

    assert!(matches!(
        broker.check(child, &net(&["a.example.com"])),
        Err(helix_runtime::capability::CapabilityError::Revoked { .. })
    ));
    // The parent is also revoked.
    assert!(matches!(
        broker.check(parent, &net(&["a.example.com"])),
        Err(helix_runtime::capability::CapabilityError::Revoked { .. })
    ));
}

/// A token minted by one broker must never resolve in a different broker
/// instance (unforgeability / isolation property).
#[test]
fn tokens_are_not_portable_across_brokers() {
    let mut a = CapabilityBroker::new();
    let b = CapabilityBroker::new();
    let tok = a.grant("app".into(), net(&["a.example.com"]));
    assert!(b.resolve(tok).is_none());
    assert!(matches!(
        b.check(tok, &net(&["a.example.com"])),
        Err(helix_runtime::capability::CapabilityError::InvalidToken(_))
    ));
}
