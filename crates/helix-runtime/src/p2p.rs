//! Content-addressed distribution — DHT + bitswap resolution layer (spec G5,
//! Phase 1 "Content-addressed distribution").
//!
//! `content.rs` owns the *local* block store + integrity checks. This module
//! owns the *network* half: turning a [`ContentId`] into bytes by asking the
//! swarm. It models the libp2p IPFS stack's two primitives:
//!
//! * **DHT `provide`/`findproviders`** — announce that a peer holds a block and
//!   discover which peers hold a given `ContentId` (content routing).
//! * **bitswap** — pull the actual block bytes from a provider, verifying the
//!   SHA-256 against the requested id on receipt (content *retrieval*).
//!
//! The [`ContentSource`] trait is the seam where a real `libp2p`-backed
//! transport plugs in. [`PeerNetwork`] is an in-process implementation of that
//! transport used by the runtime and by tests; it exercises exactly the same
//! provide → query → fetch-with-integrity path a live DHT/bitswap would, so the
//! resolution logic is validated without requiring a reachable P2P network.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::content::{ContentId, ContentStore, digest};

/// A peer in the content swarm, identified by an opaque multiaddr-like string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerId(pub String);

impl PeerId {
    pub fn new(s: impl Into<String>) -> Self {
        PeerId(s.into())
    }
}

/// A provider record returned by content routing (`findproviders`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub peer: PeerId,
    /// Approximate byte size advertised by the provider (0 = unknown).
    pub size: u64,
}

/// Errors from the content resolution / retrieval path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentSourceError {
    /// No provider was found for the content id (DHT miss).
    NotFound(ContentId),
    /// A provider answered but the returned bytes failed integrity verification.
    Integrity(ContentId),
    /// The fetch could not be completed (provider unreachable / empty).
    Unavailable(ContentId),
}

/// A content-addressable block source: content routing + retrieval.
///
/// This trait is the single seam between the runtime and the network transport.
/// The default in-process [`PeerNetwork`] implements it; a `libp2p`-backed
/// transport implements the same contract over a real Kademlia DHT + bitswap
/// session (see module docs).
pub trait ContentSource {
    /// Content routing: list peers that have announced `id`.
    fn find_providers(&self, id: &ContentId) -> Vec<Provider>;

    /// Retrieval: fetch the bytes for `id` from `provider`, verifying the
    /// content hash on receipt. Returns the verified bytes, or an error.
    fn fetch_block(
        &self,
        provider: &Provider,
        id: &ContentId,
    ) -> Result<Vec<u8>, ContentSourceError>;

    /// Convenience: resolve-and-fetch from any known provider, returning the
    /// first verified block (or the first error if none succeed).
    fn get(&self, id: &ContentId) -> Result<Vec<u8>, ContentSourceError> {
        let providers = self.find_providers(id);
        if providers.is_empty() {
            return Err(ContentSourceError::NotFound(id.clone()));
        }
        let mut last_err = ContentSourceError::Unavailable(id.clone());
        for provider in &providers {
            match self.fetch_block(provider, id) {
                Ok(bytes) => return Ok(bytes),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

/// An in-process content swarm: peers announce blocks they hold and serve
/// fetched bytes with integrity checks. Mirrors the DHT/bitswap contract.
#[derive(Clone, Default)]
pub struct PeerNetwork {
    inner: Arc<Mutex<PeerState>>,
}

#[derive(Default)]
struct PeerState {
    /// peer -> the content store it advertises (bitswap "have" set).
    stores: HashMap<PeerId, ContentStore>,
    /// content id -> set of peers that have announced it (DHT index).
    routing: HashMap<ContentId, HashSet<PeerId>>,
}

impl PeerNetwork {
    pub fn new() -> Self {
        PeerNetwork::default()
    }

    /// Register a peer with an empty store (e.g. a freshly-connected libp2p node).
    pub fn join(&self, peer: PeerId) {
        self.inner.lock().unwrap().stores.entry(peer).or_default();
    }

    /// `provide`: a peer announces it holds `bytes`, returning the assigned id.
    /// This is the DHT `provide` step plus the local bitswap `have`.
    pub fn provide(&self, peer: &PeerId, bytes: &[u8]) -> ContentId {
        let mut st = self.inner.lock().unwrap();
        let id = st.stores.entry(peer.clone()).or_default().put(bytes);
        st.routing
            .entry(id.clone())
            .or_default()
            .insert(peer.clone());
        id
    }

    /// `findproviders`: content routing lookup.
    pub fn providers_of(&self, id: &ContentId) -> Vec<Provider> {
        let st = self.inner.lock().unwrap();
        match st.routing.get(id) {
            Some(peers) => peers
                .iter()
                .map(|p| {
                    let size = st
                        .stores
                        .get(p)
                        .and_then(|s| s.get(id))
                        .map(|b| b.len() as u64)
                        .unwrap_or(0);
                    Provider {
                        peer: p.clone(),
                        size,
                    }
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// `bitswap` fetch: pull a block from a specific peer, verifying the hash.
    pub fn fetch_from(
        &self,
        provider: &PeerId,
        id: &ContentId,
    ) -> Result<Vec<u8>, ContentSourceError> {
        let st = self.inner.lock().unwrap();
        let bytes = st
            .stores
            .get(provider)
            .and_then(|s| s.get_verified(id))
            .ok_or_else(|| ContentSourceError::Unavailable(id.clone()))?;
        if ContentStore::verify(id, &bytes) {
            Ok(bytes)
        } else {
            Err(ContentSourceError::Integrity(id.clone()))
        }
    }

    /// Resolve-and-fetch across all known providers (see [`ContentSource::get`]).
    pub fn resolve(&self, id: &ContentId) -> Result<Vec<u8>, ContentSourceError> {
        self.get(id)
    }
}

impl ContentSource for PeerNetwork {
    fn find_providers(&self, id: &ContentId) -> Vec<Provider> {
        self.providers_of(id)
    }

    fn fetch_block(
        &self,
        provider: &Provider,
        id: &ContentId,
    ) -> Result<Vec<u8>, ContentSourceError> {
        self.fetch_from(&provider.peer, id)
    }
}

/// A [`ContentSource`] adapter that layers DHT/bitswap retrieval on top of a
/// local [`ContentStore`] fallback: a `ContentId` not found in the swarm is
/// served from the local cache. This is how the runtime satisfies a fetch when
/// the asset was previously cached (offline-first, G9).
pub struct LocalFallbackSource {
    network: PeerNetwork,
    local: Arc<Mutex<ContentStore>>,
}

impl LocalFallbackSource {
    pub fn new(network: PeerNetwork) -> Self {
        LocalFallbackSource {
            network,
            local: Arc::new(Mutex::new(ContentStore::new())),
        }
    }

    /// Seed the local cache (e.g. from a packaged/offline content bundle).
    pub fn cache(&self, bytes: &[u8]) -> ContentId {
        self.local.lock().unwrap().put(bytes)
    }
}

impl ContentSource for LocalFallbackSource {
    fn find_providers(&self, id: &ContentId) -> Vec<Provider> {
        let mut providers = self.network.find_providers(id);
        if providers.is_empty() && self.local.lock().unwrap().get_verified(id).is_some() {
            providers.push(Provider {
                peer: PeerId::new("local-cache"),
                size: self
                    .local
                    .lock()
                    .unwrap()
                    .get(id)
                    .map(|b| b.len() as u64)
                    .unwrap_or(0),
            });
        }
        providers
    }

    fn fetch_block(
        &self,
        provider: &Provider,
        id: &ContentId,
    ) -> Result<Vec<u8>, ContentSourceError> {
        if provider.peer == PeerId::new("local-cache") {
            self.local
                .lock()
                .unwrap()
                .get_verified(id)
                .ok_or_else(|| ContentSourceError::Unavailable(id.clone()))
        } else {
            self.network.fetch_block(provider, id)
        }
    }
}

/// Helper: compute a `ContentId` for bytes (re-exported convenience).
pub fn content_id(bytes: &[u8]) -> ContentId {
    digest(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provide_then_resolve_via_dht_bitswap() {
        let net = PeerNetwork::new();
        let peer = PeerId::new("peer-a");
        net.join(peer.clone());
        let id = net.provide(&peer, b"immutable video segment");
        // DHT routing resolves the provider.
        let providers = net.find_providers(&id);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].peer, peer);
        // bitswap fetch returns integrity-verified bytes.
        let bytes = net.fetch_from(&peer, &id).unwrap();
        assert_eq!(bytes, b"immutable video segment");
    }

    #[test]
    fn missing_content_is_not_found() {
        let net = PeerNetwork::new();
        let id = digest(b"nobody has this");
        assert!(matches!(
            net.resolve(&id),
            Err(ContentSourceError::NotFound(_))
        ));
    }

    #[test]
    fn multiple_providers_all_serve_verified_bytes() {
        let net = PeerNetwork::new();
        let a = PeerId::new("a");
        let b = PeerId::new("b");
        net.join(a.clone());
        net.join(b.clone());
        let id = net.provide(&a, b"shared");
        net.provide(&b, b"shared");
        let providers = net.find_providers(&id);
        assert_eq!(providers.len(), 2);
        for p in providers {
            assert_eq!(net.fetch_block(&p, &id).unwrap(), b"shared");
        }
    }

    #[test]
    fn local_fallback_serves_cached_content() {
        let net = PeerNetwork::new();
        let src = LocalFallbackSource::new(net);
        let id = src.cache(b"offline asset");
        assert!(matches!(
            src.find_providers(&id).as_slice(),
            [Provider { peer, .. }] if *peer == PeerId::new("local-cache")
        ));
        assert_eq!(src.get(&id).unwrap(), b"offline asset");
    }
}
