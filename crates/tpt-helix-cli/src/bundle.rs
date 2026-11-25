//! Signed, content-addressed app bundles (spec §10.4).
//!
//! A bundle is a directory with a `bundle.json` next to the app's files. The
//! manifest lists every other file with its SHA-256, plus the app name and
//! version. An Ed25519 signature covers the canonical manifest bytes, and the
//! SHA-256 of those bytes is the bundle's content id. Verification needs no
//! network. The caller supplies the trusted public key. A key embedded in the
//! bundle is never trusted on its own.

use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CliError;

/// The file that holds the manifest, relative to the bundle root.
pub const MANIFEST_FILE: &str = "bundle.json";
/// The only manifest format this version reads and writes.
pub const FORMAT: &str = "helix-bundle/1";

/// One file in the bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path relative to the bundle root, with `/` separators.
    pub path: String,
    /// Lower-case hex SHA-256 of the file's bytes.
    pub sha256: String,
}

/// The signed part of a bundle. Field order fixes the canonical encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub app: String,
    pub version: String,
    /// Sorted by path.
    pub files: Vec<FileEntry>,
}

/// The on-disk `bundle.json`: the manifest plus the signing key and signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleFile {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// Hex Ed25519 public key that produced `signature`.
    pub public_key: String,
    /// Hex Ed25519 signature over `canonical(manifest)`.
    pub signature: String,
}

/// Hex-encodes `bytes` in lower case.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes a hex string. Returns `None` on odd length or a non-hex character.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// The canonical bytes the signature and content id cover. The encoding is
/// `serde_json` over the fixed struct layout, so it is stable for one version.
pub fn canonical(manifest: &Manifest) -> Vec<u8> {
    serde_json::to_vec(manifest).expect("a manifest always serializes")
}

/// The bundle's content id: the hex SHA-256 of its canonical manifest.
pub fn content_id(manifest: &Manifest) -> String {
    to_hex(&Sha256::digest(canonical(manifest)))
}

/// Every regular file under `root` except `bundle.json`, as sorted relative paths.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>, CliError> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> Result<(), CliError> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&path, root, out)?;
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .expect("walked under root")
                    .to_path_buf();
                if rel != Path::new(MANIFEST_FILE) {
                    out.push(rel);
                }
            } else {
                return Err(CliError(format!(
                    "{} is a symlink or special file; bundles hold regular files only",
                    path.display()
                )));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

fn rel_string(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Hashes every file under `root` and returns the manifest for them.
pub fn build_manifest(root: &Path, app: &str, version: &str) -> Result<Manifest, CliError> {
    let mut files = Vec::new();
    for rel in collect_files(root)? {
        let bytes = fs::read(root.join(&rel))?;
        files.push(FileEntry {
            path: rel_string(&rel),
            sha256: to_hex(&Sha256::digest(&bytes)),
        });
    }
    Ok(Manifest {
        format: FORMAT.to_string(),
        app: app.to_string(),
        version: version.to_string(),
        files,
    })
}

/// The public key (hex) for the Ed25519 seed `seed_hex`.
pub fn public_key_hex(seed_hex: &str) -> Result<String, CliError> {
    let seed = from_hex(seed_hex.trim())
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| CliError("the key must be a 32-byte Ed25519 seed in hex".into()))?;
    Ok(to_hex(
        SigningKey::from_bytes(&seed).verifying_key().as_bytes(),
    ))
}

/// Signs the bundle in `root` with the Ed25519 seed `seed_hex` (32 bytes, hex)
/// and writes `bundle.json`. Returns the content id.
pub fn sign_bundle(
    root: &Path,
    app: &str,
    version: &str,
    seed_hex: &str,
) -> Result<String, CliError> {
    let seed = from_hex(seed_hex.trim())
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| CliError("the key must be a 32-byte Ed25519 seed in hex".into()))?;
    let key = SigningKey::from_bytes(&seed);
    let manifest = build_manifest(root, app, version)?;
    let signature = key.sign(&canonical(&manifest));
    let bundle = BundleFile {
        manifest: manifest.clone(),
        public_key: to_hex(key.verifying_key().as_bytes()),
        signature: to_hex(&signature.to_bytes()),
    };
    let json = serde_json::to_string_pretty(&bundle)
        .map_err(|e| CliError(format!("cannot encode {MANIFEST_FILE}: {e}")))?;
    fs::write(root.join(MANIFEST_FILE), json + "\n")?;
    Ok(content_id(&manifest))
}

/// What verification found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub app: String,
    pub version: String,
    pub content_id: String,
}

/// Verifies the bundle in `root` against the trusted public key `trust_hex`.
///
/// Checks, in order: the format, the signature under `trust_hex`, that the
/// embedded key equals the trusted key, that every listed file exists with the
/// listed hash, and that no unlisted file is present.
pub fn verify_bundle(root: &Path, trust_hex: &str) -> Result<Verified, CliError> {
    let text = fs::read_to_string(root.join(MANIFEST_FILE))
        .map_err(|e| CliError(format!("cannot read {MANIFEST_FILE}: {e}")))?;
    let bundle: BundleFile = serde_json::from_str(&text)
        .map_err(|e| CliError(format!("{MANIFEST_FILE} is not a bundle manifest: {e}")))?;
    let m = &bundle.manifest;
    if m.format != FORMAT {
        return Err(CliError(format!(
            "unsupported bundle format `{}` (expected {FORMAT})",
            m.format
        )));
    }

    let trusted = from_hex(trust_hex.trim())
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| {
            CliError("the trusted key must be a 32-byte Ed25519 public key in hex".into())
        })?;
    let trusted_key = VerifyingKey::from_bytes(&trusted)
        .map_err(|e| CliError(format!("invalid trusted key: {e}")))?;
    if bundle.public_key.trim() != trust_hex.trim() {
        return Err(CliError(
            "the bundle is signed by a key that is not the trusted key".into(),
        ));
    }
    let sig_bytes = from_hex(&bundle.signature)
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
        .ok_or_else(|| CliError("the signature is not 64 hex-encoded bytes".into()))?;
    trusted_key
        .verify(&canonical(m), &Signature::from_bytes(&sig_bytes))
        .map_err(|_| CliError("the signature does not match the manifest".into()))?;

    let mut listed: Vec<&str> = Vec::new();
    for entry in &m.files {
        let path = root.join(&entry.path);
        let bytes = fs::read(&path)
            .map_err(|e| CliError(format!("listed file `{}` cannot be read: {e}", entry.path)))?;
        if to_hex(&Sha256::digest(&bytes)) != entry.sha256 {
            return Err(CliError(format!("`{}` has been modified", entry.path)));
        }
        listed.push(&entry.path);
    }
    let on_disk = collect_files(root)?;
    for rel in on_disk {
        let rel = rel_string(&rel);
        if !listed.contains(&rel.as_str()) {
            return Err(CliError(format!("`{rel}` is not in the signed manifest")));
        }
    }

    Ok(Verified {
        app: m.app.clone(),
        version: m.version.clone(),
        content_id: content_id(m),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn public_key_for(seed_hex: &str) -> String {
        let seed = <[u8; 32]>::try_from(from_hex(seed_hex).unwrap()).unwrap();
        to_hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes())
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("helix-bundle-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("index.html"), "<h1>hi</h1>").unwrap();
        fs::write(dir.join("assets/app.css"), "h1 { color: red; }").unwrap();
        dir
    }

    #[test]
    fn a_signed_bundle_verifies_against_its_trusted_key() {
        let dir = scratch("ok");
        let id = sign_bundle(&dir, "notes", "1.0.0", SEED).unwrap();
        let v = verify_bundle(&dir, &public_key_for(SEED)).expect("verifies");
        assert_eq!(v.app, "notes");
        assert_eq!(v.version, "1.0.0");
        assert_eq!(v.content_id, id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_modified_file_is_detected() {
        let dir = scratch("modified");
        sign_bundle(&dir, "notes", "1.0.0", SEED).unwrap();
        fs::write(dir.join("index.html"), "<h1>changed</h1>").unwrap();
        let err = verify_bundle(&dir, &public_key_for(SEED)).unwrap_err();
        assert!(err.to_string().contains("index.html"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_added_file_is_detected() {
        let dir = scratch("added");
        sign_bundle(&dir, "notes", "1.0.0", SEED).unwrap();
        fs::write(dir.join("extra.js"), "alert(1)").unwrap();
        let err = verify_bundle(&dir, &public_key_for(SEED)).unwrap_err();
        assert!(err.to_string().contains("extra.js"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_signed_by_another_key_is_rejected() {
        let dir = scratch("otherkey");
        sign_bundle(&dir, "notes", "1.0.0", SEED).unwrap();
        let other = "0202020202020202020202020202020202020202020202020202020202020202";
        let err = verify_bundle(&dir, &public_key_for(other)).unwrap_err();
        assert!(err.to_string().contains("not the trusted key"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_embedded_key_cannot_replace_the_trusted_one() {
        // A forger re-signs the files with their own key and embeds it. Verifying
        // against the real trusted key must still fail.
        let dir = scratch("forged");
        sign_bundle(&dir, "notes", "1.0.0", SEED).unwrap();
        let forged = "0303030303030303030303030303030303030303030303030303030303030303";
        sign_bundle(&dir, "notes", "9.9.9", forged).unwrap();
        let err = verify_bundle(&dir, &public_key_for(SEED)).unwrap_err();
        assert!(err.to_string().contains("not the trusted key"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampering_with_the_manifest_breaks_the_signature() {
        let dir = scratch("tamper");
        sign_bundle(&dir, "notes", "1.0.0", SEED).unwrap();
        let text = fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        fs::write(dir.join(MANIFEST_FILE), text.replace("1.0.0", "2.0.0")).unwrap();
        let err = verify_bundle(&dir, &public_key_for(SEED)).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_id_is_stable_for_the_same_files() {
        let dir = scratch("stable");
        let a = build_manifest(&dir, "notes", "1.0.0").unwrap();
        let b = build_manifest(&dir, "notes", "1.0.0").unwrap();
        assert_eq!(content_id(&a), content_id(&b));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_key_is_refused() {
        let dir = scratch("badkey");
        assert!(sign_bundle(&dir, "notes", "1.0.0", "abcd").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
