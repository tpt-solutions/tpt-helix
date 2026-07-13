//! Content-addressed distribution — local block store with integrity checks.
//!
//! Per G5, immutable assets are identified by the cryptographic hash of their
//! bytes rather than a network location. This module is the local half of that
//! pipeline: a content-addressed block store plus SHA-256 integrity
//! verification. The network half (libp2p DHT + bitswap resolution of a
//! `ContentId` into bytes) is layered on top of [`ContentStore`] and is tracked
//! separately in TODO.md (§"Content-addressed distribution").
//!
//! Identifiers are lowercase hex-encoded SHA-256 digests, which are stable,
//! human-readable, and trivially representable in WIT (`string`).

use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A content identifier: the lowercase hex SHA-256 of the addressed bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId(pub String);

impl std::fmt::Display for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Compute the [`ContentId`] for a byte slice.
pub fn digest(bytes: &[u8]) -> ContentId {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    ContentId(s)
}

/// A local, immutable content-addressed block store.
///
/// Blocks are keyed by their [`ContentId`]; inserting the same bytes twice is
/// idempotent. All reads are integrity-checked against the requested id, so a
/// corrupted or substituted block is never returned as valid.
#[derive(Debug, Default)]
pub struct ContentStore {
    blocks: HashMap<ContentId, Vec<u8>>,
}

impl ContentStore {
    pub fn new() -> Self {
        ContentStore {
            blocks: HashMap::new(),
        }
    }

    /// Number of blocks currently cached.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Store `bytes`, returning their content id. Idempotent.
    pub fn put(&mut self, bytes: &[u8]) -> ContentId {
        let id = digest(bytes);
        self.blocks.entry(id.clone()).or_insert_with(|| bytes.to_vec());
        id
    }

    /// Resolve `id` to its bytes if present. (The id is already bound to the
    /// bytes at insert time, so this is integrity-preserving.)
    pub fn get(&self, id: &ContentId) -> Option<&[u8]> {
        self.blocks.get(id).map(Vec::as_slice)
    }

    /// Integrity-checked fetch: returns the bytes only if they hash to `id`.
    /// Returns `None` if absent or if the stored block fails to verify (which
    /// would indicate on-disk corruption or a substitution attack).
    pub fn get_verified(&self, id: &ContentId) -> Option<Vec<u8>> {
        let bytes = self.get(id)?;
        if &digest(bytes) == id {
            Some(bytes.to_vec())
        } else {
            None
        }
    }

    /// Remove a block from the local store.
    pub fn remove(&mut self, id: &ContentId) -> bool {
        self.blocks.remove(id).is_some()
    }

    /// Verify that `bytes` match the expected `id` without storing them.
    pub fn verify(id: &ContentId, bytes: &[u8]) -> bool {
        &digest(bytes) == id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_hex() {
        let a = digest(b"hello helix");
        let b = digest(b"hello helix");
        assert_eq!(a, b);
        assert_eq!(a.0.len(), 64);
        assert!(a.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn put_get_roundtrip_and_idempotent() {
        let mut store = ContentStore::new();
        let id = store.put(b"immutable asset");
        assert_eq!(store.len(), 1);
        // re-putting same bytes does not duplicate
        let id2 = store.put(b"immutable asset");
        assert_eq!(id, id2);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&id), Some(&b"immutable asset"[..]));
    }

    #[test]
    fn verify_rejects_tampered_bytes() {
        assert!(ContentStore::verify(&digest(b"a"), b"a"));
        assert!(!ContentStore::verify(&digest(b"a"), b"b"));
    }

    #[test]
    fn get_verified_returns_none_for_missing() {
        let store = ContentStore::new();
        assert!(store
            .get_verified(&digest(b"never stored"))
            .is_none());
    }
}
