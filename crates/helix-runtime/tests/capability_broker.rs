//! Integration tests: the Capability Broker enforced through the runtime's
//! capability-aware `RuntimeState` (the same path the wasmtime `Host` and the
//! `RuntimeStub` delegate to). Covers grant scoping, deny, revocation/abort,
//! and inter-module delegation.

use helix_runtime::capability::{
    Capability, CapabilityBroker, DomScope, HostPattern, StorageScope,
};
use helix_runtime::stub::{Request, Response, RuntimeState, VideoConfig};

fn state_with(app: &str, grants: &[Capability]) -> RuntimeState {
    let broker = CapabilityBroker::new();
    let mut state = RuntimeState::with_broker(app.to_string(), broker);
    for g in grants {
        state.grant(g.clone());
    }
    state
}

#[test]
fn network_grant_scoped_to_host_allows_then_denies() {
    let mut state = state_with(
        "app",
        &[Capability::Network {
            hosts: vec![HostPattern::Exact("api.example.com".into())],
        }],
    );

    let ok = Request {
        method: "GET".into(),
        url: "https://api.example.com/data".into(),
        headers: vec![],
        body: None,
    };
    state.register_fetch(
        "https://api.example.com/data",
        Response {
            status: 200,
            headers: vec![],
            body: b"ok".to_vec(),
        },
    );
    assert!(state.fetch(ok).is_ok());

    let denied = Request {
        method: "GET".into(),
        url: "https://evil.test/".into(),
        headers: vec![],
        body: None,
    };
    assert!(state.fetch(denied).is_err());
}

#[test]
fn storage_grant_scoped_to_namespace() {
    let mut state = state_with(
        "app",
        &[Capability::Storage {
            scope: StorageScope::Namespace("app:notes:".into()),
        }],
    );

    assert!(state.set("app:notes:1".into(), b"v".to_vec()).is_ok());
    assert_eq!(state.get("app:notes:1".into()), Some(b"v".to_vec()));

    // Writing outside the granted namespace is denied.
    assert!(state.set("other:1".into(), b"x".to_vec()).is_err());
    assert!(state.get("other:1".into()).is_none());
}

#[test]
fn dom_grant_required_for_element_ops() {
    let mut granted = state_with(
        "app",
        &[Capability::Dom {
            scope: DomScope::Full,
        }],
    );
    let id = granted.create_element("div".into());
    assert_ne!(id, u64::MAX);
    granted.set_text(id, "hi".into());
    assert_eq!(granted.element(id).unwrap().text, "hi");

    // Without a dom grant, element ops are denied (no-op / unusable handle).
    let mut denied = state_with("app", &[]);
    let bad = denied.create_element("div".into());
    assert_eq!(bad, u64::MAX);
}

#[test]
fn revocation_aborts_further_capability_use() {
    let mut state = RuntimeState::with_broker("app".into(), CapabilityBroker::new());
    let token = state.grant(Capability::Network {
        hosts: vec![HostPattern::Exact("api.example.com".into())],
    });
    state.register_fetch(
        "https://api.example.com/",
        Response {
            status: 200,
            headers: vec![],
            body: b"ok".to_vec(),
        },
    );

    let req = Request {
        method: "GET".into(),
        url: "https://api.example.com/".into(),
        headers: vec![],
        body: None,
    };
    // Authorized via the grant recorded by `state.grant`.
    assert!(state.fetch(req.clone()).is_ok());

    // Revoke it (simulating the user pulling access at runtime).
    let gid = state.broker().unwrap().resolve(token).unwrap();
    assert!(state.broker_mut().unwrap().revoke(gid));

    // Subsequent use is denied — the module would trap/abort here.
    assert!(state.fetch(req).is_err());
}

#[test]
fn delegation_flows_capability_to_second_module() {
    let mut broker = CapabilityBroker::new();
    let parent = broker.grant(
        "parent".into(),
        Capability::Network {
            hosts: vec![HostPattern::Suffix(".example.com".into())],
        },
    );

    // Parent delegates a narrower network capability to a child module.
    let child = broker
        .delegate(
            parent,
            "child".into(),
            Capability::Network {
                hosts: vec![HostPattern::Exact("api.example.com".into())],
            },
        )
        .expect("delegation within parent scope");

    // Child may use the delegated, narrowed host.
    assert!(
        broker
            .check(
                child,
                &Capability::Network {
                    hosts: vec![HostPattern::Exact("api.example.com".into())]
                }
            )
            .is_ok()
    );

    // Child cannot exceed the parent's scope.
    assert!(
        broker
            .delegate(
                parent,
                "child".into(),
                Capability::Network {
                    hosts: vec![HostPattern::Exact("other.test".into())]
                }
            )
            .is_err()
    );
}

#[test]
fn media_capability_enforces_resolution_cap() {
    let mut state = state_with(
        "app",
        &[Capability::Media {
            max_resolution: Some((1280, 720)),
        }],
    );

    // 480p is within the granted cap.
    let ok = state
        .create_player(VideoConfig {
            codec: "h264".into(),
            width: 640,
            height: 480,
            bitrate: 1_000_000,
        })
        .expect("480p allowed");
    assert!(state.player(ok).is_some());

    // 1080p exceeds the granted cap → denied (trap/abort).
    let denied = state.create_player(VideoConfig {
        codec: "h264".into(),
        width: 1920,
        height: 1080,
        bitrate: 4_000_000,
    });
    assert!(denied.is_err());
}

#[test]
fn media_player_lifecycle_play_pause_seek() {
    let mut state = RuntimeState::with_broker("app".into(), CapabilityBroker::new());
    // No media grant in legacy-permissive mode (broker present but no grant):
    // create_player is denied until a grant is recorded for the app.
    assert!(
        state
            .create_player(VideoConfig {
                codec: "vp9".into(),
                width: 320,
                height: 240,
                bitrate: 500_000,
            })
            .is_err()
    );

    state.grant(Capability::Media {
        max_resolution: Some((1920, 1080)),
    });
    let id = state
        .create_player(VideoConfig {
            codec: "vp9".into(),
            width: 320,
            height: 240,
            bitrate: 500_000,
        })
        .expect("player created after grant");
    assert!(!state.player(id).unwrap().playing);

    state.play(id);
    assert!(state.player(id).unwrap().playing);
    state.seek(id, 42_000);
    assert_eq!(state.player(id).unwrap().position_ms, 42_000);
    state.pause(id);
    assert!(!state.player(id).unwrap().playing);
}
