//! Capability registry — content-addressed discovery, publishing, and fetching.
//!
//! Registry entries are signed capability manifests with a CID pointing to the
//! WASM component. The registry is a content-addressed document graph — no
//! central database. Entries are stored locally and can be synchronized via
//! libp2p pubsub.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::artifacts::Cid;
use crate::error::ManifestError;
use crate::manifest::SignedManifest;

/// Errors returned when publishing to the registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The accompanying signed manifest failed verification.
    #[error("manifest verification failed: {0}")]
    Manifest(#[from] ManifestError),

    /// The entry's declared author does not match the signed manifest author.
    #[error("author mismatch: entry claims {entry}, signed manifest is authored by {manifest}")]
    AuthorMismatch {
        /// The author declared by the registry entry.
        entry: String,
        /// The author authenticated by the signed manifest.
        manifest: String,
    },

    /// A capability with this name and version is already published.
    #[error("{0} already published")]
    DuplicateVersion(String),

    /// Underlying I/O failure while persisting the registry.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A published capability in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Capability name (e.g., "gang-capability-diagnostics").
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// Short description.
    pub description: String,
    /// Author's peer ID.
    pub author_peer_id: String,
    /// Language the capability was authored in.
    pub language: CapabilityLanguage,
    /// CID of the signed WASM component.
    pub component_cid: Cid,
    /// CID of the signed manifest.
    pub manifest_cid: Cid,
    /// Declared capability groups.
    pub declared_capabilities: Vec<String>,
    /// When this entry was published (ISO 8601).
    pub published_at: String,
    /// Tags for discoverability.
    pub tags: Vec<String>,
    /// Minimum Ganglion version required.
    pub min_ganglion_version: Option<String>,
}

/// The language a capability was authored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum CapabilityLanguage {
    /// Rust.
    Rust,
    /// C++.
    Cpp,
    /// Python.
    Python,
    /// Go.
    Go,
    /// Any other or unspecified language.
    #[default]
    Other,
}

impl std::fmt::Display for CapabilityLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "Rust"),
            Self::Cpp => write!(f, "C++"),
            Self::Python => write!(f, "Python"),
            Self::Go => write!(f, "Go"),
            Self::Other => write!(f, "Other"),
        }
    }
}

/// Local capability registry backed by a JSON file.
pub struct Registry {
    /// Path to the registry index file.
    index_path: PathBuf,
    /// In-memory index of entries, keyed by name.
    entries: HashMap<String, Vec<RegistryEntry>>,
}

/// Search results from the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Capability name.
    pub name: String,
    /// Highest published semantic version.
    pub latest_version: String,
    /// Short description.
    pub description: String,
    /// Author peer ID.
    pub author: String,
    /// Authoring language.
    pub language: CapabilityLanguage,
    /// Discoverability tags.
    pub tags: Vec<String>,
}

/// Parse a version string into a comparable `(major, minor, patch)` tuple.
///
/// `semver` is not available in the workspace dependency table, so this does a
/// lightweight parse of the `major.minor.patch` core, ignoring any
/// pre-release/build suffix (after `-` or `+`). Missing or non-numeric
/// components are treated as `0`, which keeps ordering total and predictable
/// for malformed input.
fn parse_semver(version: &str) -> (u64, u64, u64) {
    // Drop build/pre-release metadata; compare only the numeric core.
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Select the entry with the highest semantic version, breaking ties by the
/// later publish order (later insertion wins).
fn latest_entry(versions: &[RegistryEntry]) -> Option<&RegistryEntry> {
    versions
        .iter()
        .enumerate()
        .max_by_key(|(idx, e)| (parse_semver(&e.version), *idx))
        .map(|(_, e)| e)
}

impl Registry {
    /// Open or create a registry at the given directory.
    pub fn open(registry_dir: &Path) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(registry_dir)?;
        let index_path = registry_dir.join("registry.json");

        let entries = if index_path.exists() {
            let data = std::fs::read_to_string(&index_path)?;
            serde_json::from_str(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
        } else {
            HashMap::new()
        };

        Ok(Self {
            index_path,
            entries,
        })
    }

    /// Publish a capability to the registry.
    ///
    /// The caller must supply the [`SignedManifest`] that backs the entry. The
    /// manifest signature is verified (reusing [`SignedManifest::verify_and_decode`],
    /// which also confirms the manifest's author peer ID matches the signing
    /// key) and the entry's declared `author_peer_id` must match the verified
    /// manifest author. Publishing is rejected on any mismatch, preventing an
    /// entry from being attributed to a peer that did not sign the manifest.
    pub fn publish(
        &mut self,
        entry: RegistryEntry,
        signed_manifest: &SignedManifest,
    ) -> Result<(), RegistryError> {
        // Verify the signed manifest and recover the authenticated author.
        let manifest = signed_manifest.verify_and_decode()?;

        if entry.author_peer_id != manifest.author_peer_id.as_str() {
            return Err(RegistryError::AuthorMismatch {
                entry: entry.author_peer_id.clone(),
                manifest: manifest.author_peer_id.to_string(),
            });
        }

        let versions = self.entries.entry(entry.name.clone()).or_default();

        // Check for duplicate version
        if versions.iter().any(|e| e.version == entry.version) {
            return Err(RegistryError::DuplicateVersion(format!(
                "{}@{}",
                entry.name, entry.version
            )));
        }

        versions.push(entry);
        self.persist()?;
        Ok(())
    }

    /// Search for capabilities by query string.
    /// Matches against name, description, and tags.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for (name, versions) in &self.entries {
            if let Some(latest) = latest_entry(versions) {
                let matches = name.to_lowercase().contains(&query_lower)
                    || latest.description.to_lowercase().contains(&query_lower)
                    || latest
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower));

                if matches {
                    results.push(SearchResult {
                        name: name.clone(),
                        latest_version: latest.version.clone(),
                        description: latest.description.clone(),
                        author: latest.author_peer_id.clone(),
                        language: latest.language,
                        tags: latest.tags.clone(),
                    });
                }
            }
        }

        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Get all versions of a capability by name.
    pub fn get(&self, name: &str) -> Option<&[RegistryEntry]> {
        self.entries.get(name).map(|v| v.as_slice())
    }

    /// Get the latest version of a capability (highest semantic version).
    pub fn get_latest(&self, name: &str) -> Option<&RegistryEntry> {
        self.entries.get(name).and_then(|v| latest_entry(v))
    }

    /// List all capabilities in the registry.
    pub fn list(&self) -> Vec<SearchResult> {
        let mut results = Vec::new();
        for (name, versions) in &self.entries {
            if let Some(latest) = latest_entry(versions) {
                results.push(SearchResult {
                    name: name.clone(),
                    latest_version: latest.version.clone(),
                    description: latest.description.clone(),
                    author: latest.author_peer_id.clone(),
                    language: latest.language,
                    tags: latest.tags.clone(),
                });
            }
        }
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Remove a capability (all versions) from the registry.
    pub fn remove(&mut self, name: &str) -> Result<bool, std::io::Error> {
        let existed = self.entries.remove(name).is_some();
        if existed {
            self.persist()?;
        }
        Ok(existed)
    }

    /// Number of capabilities in the registry.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    fn persist(&self) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(&self.entries).map_err(std::io::Error::other)?;
        std::fs::write(&self.index_path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::identity::Keypair;
    use crate::manifest::{ComponentManifest, MANIFEST_SCHEMA_VERSION, ResourceLimits};

    fn sample_entry(name: &str, version: &str) -> RegistryEntry {
        RegistryEntry {
            name: name.into(),
            version: version.into(),
            description: format!("A {name} capability"),
            author_peer_id: "12D3KooWTestPeer".into(),
            language: CapabilityLanguage::Rust,
            component_cid: Cid::from_bytes(format!("{name}-{version}-component").as_bytes()),
            manifest_cid: Cid::from_bytes(format!("{name}-{version}-manifest").as_bytes()),
            declared_capabilities: vec!["ganglion:diagnostics/collect".into()],
            published_at: "2026-04-23T12:00:00Z".into(),
            tags: vec![name.replace("gang-capability-", ""), "system".into()],
            min_ganglion_version: Some("0.4.0".into()),
        }
    }

    /// Build an entry plus a matching signed manifest authored by `kp`.
    fn signed_entry(kp: &Keypair, name: &str, version: &str) -> (RegistryEntry, SignedManifest) {
        let manifest = ComponentManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.into(),
            name: name.into(),
            version: version.into(),
            declared_capabilities: vec![],
            author_peer_id: kp.peer_id(),
            component_hash: blake3::hash(format!("{name}-{version}").as_bytes())
                .to_hex()
                .to_string(),
            limits: ResourceLimits::default(),
            language: CapabilityLanguage::Rust,
            description: String::new(),
            tags: vec![],
            min_ganglion_version: None,
        };
        let signed = SignedManifest::sign(&manifest, kp).unwrap();

        let mut entry = sample_entry(name, version);
        entry.author_peer_id = kp.peer_id().to_string();
        (entry, signed)
    }

    /// Publish a signed sample entry, asserting success.
    fn publish_ok(reg: &mut Registry, kp: &Keypair, name: &str, version: &str) {
        let (entry, signed) = signed_entry(kp, name, version);
        reg.publish(entry, &signed).unwrap();
    }

    #[test]
    fn publish_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        publish_ok(&mut reg, &kp, "gang-capability-diagnostics", "1.0.0");
        publish_ok(&mut reg, &kp, "gang-capability-param-inspect", "1.0.0");

        let results = reg.search("diagnostics");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "gang-capability-diagnostics");
    }

    #[test]
    fn publish_rejects_author_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();

        let signer = Keypair::generate();
        let (mut entry, signed) = signed_entry(&signer, "spoofed-cap", "1.0.0");
        // Claim a different author than the one who actually signed.
        entry.author_peer_id = Keypair::generate().peer_id().to_string();

        let result = reg.publish(entry, &signed);
        assert!(matches!(result, Err(RegistryError::AuthorMismatch { .. })));
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn publish_rejects_tampered_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();

        let signer = Keypair::generate();
        let (entry, mut signed) = signed_entry(&signer, "tampered-cap", "1.0.0");
        // Corrupt the signed manifest bytes so verification fails.
        if let Some(b) = signed.manifest_cbor.last_mut() {
            *b ^= 0xFF;
        }

        let result = reg.publish(entry, &signed);
        assert!(matches!(result, Err(RegistryError::Manifest(_))));
    }

    #[test]
    fn search_by_tag() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        publish_ok(&mut reg, &kp, "gang-capability-diagnostics", "1.0.0");
        publish_ok(&mut reg, &kp, "gang-capability-param-inspect", "1.0.0");

        // "system" tag is on all entries
        let results = reg.search("system");
        assert_eq!(results.len(), 2);

        // "param-inspect" tag is only on that entry
        let results = reg.search("param-inspect");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "gang-capability-param-inspect");
    }

    #[test]
    fn duplicate_version_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        publish_ok(&mut reg, &kp, "test-cap", "1.0.0");
        let (entry, signed) = signed_entry(&kp, "test-cap", "1.0.0");
        let result = reg.publish(entry, &signed);
        assert!(matches!(result, Err(RegistryError::DuplicateVersion(_))));
    }

    #[test]
    fn multiple_versions() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        publish_ok(&mut reg, &kp, "test-cap", "1.0.0");
        publish_ok(&mut reg, &kp, "test-cap", "1.1.0");

        let versions = reg.get("test-cap").unwrap();
        assert_eq!(versions.len(), 2);

        let latest = reg.get_latest("test-cap").unwrap();
        assert_eq!(latest.version, "1.1.0");
    }

    #[test]
    fn list_all() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        publish_ok(&mut reg, &kp, "cap-a", "1.0.0");
        publish_ok(&mut reg, &kp, "cap-b", "1.0.0");
        publish_ok(&mut reg, &kp, "cap-c", "1.0.0");

        let list = reg.list();
        assert_eq!(list.len(), 3);
        // Should be sorted
        assert_eq!(list[0].name, "cap-a");
        assert_eq!(list[2].name, "cap-c");
    }

    #[test]
    fn remove_capability() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        publish_ok(&mut reg, &kp, "to-remove", "1.0.0");
        assert_eq!(reg.count(), 1);

        let removed = reg.remove("to-remove").unwrap();
        assert!(removed);
        assert_eq!(reg.count(), 0);

        let not_found = reg.remove("nonexistent").unwrap();
        assert!(!not_found);
    }

    #[test]
    fn persist_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let kp = Keypair::generate();

        {
            let mut reg = Registry::open(dir.path()).unwrap();
            publish_ok(&mut reg, &kp, "persistent-cap", "1.0.0");
        }

        let reg = Registry::open(dir.path()).unwrap();
        assert_eq!(reg.count(), 1);
        let entry = reg.get_latest("persistent-cap").unwrap();
        assert_eq!(entry.version, "1.0.0");
    }

    #[test]
    fn get_latest_picks_highest_semver_out_of_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();
        let kp = Keypair::generate();

        // Publish out of order; string ordering would mis-rank these.
        publish_ok(&mut reg, &kp, "cap", "1.0.0");
        publish_ok(&mut reg, &kp, "cap", "2.0.0");
        publish_ok(&mut reg, &kp, "cap", "1.5.0");
        publish_ok(&mut reg, &kp, "cap", "0.9.0");
        publish_ok(&mut reg, &kp, "cap", "0.10.0");

        // Highest semver is 2.0.0, not the last-published (0.10.0).
        assert_eq!(reg.get_latest("cap").unwrap().version, "2.0.0");

        // search() and list() report the same latest version.
        let listed = reg.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].latest_version, "2.0.0");
        let found = reg.search("cap");
        assert_eq!(found[0].latest_version, "2.0.0");
    }

    #[test]
    fn semver_parse_orders_numerically() {
        // 0.10.0 must sort above 0.9.0 (numeric, not lexicographic).
        assert!(parse_semver("0.10.0") > parse_semver("0.9.0"));
        assert!(parse_semver("2.0.0") > parse_semver("1.99.99"));
        // Pre-release suffix is ignored for the numeric core.
        assert_eq!(parse_semver("1.2.3-rc1"), (1, 2, 3));
    }

    #[test]
    fn search_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(dir.path()).unwrap();
        let results = reg.search("anything");
        assert!(results.is_empty());
    }

    #[test]
    fn entry_json_roundtrip() {
        let entry = sample_entry("test-cap", "1.0.0");
        let json = serde_json::to_string(&entry).unwrap();
        let loaded: RegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.name, "test-cap");
        assert_eq!(loaded.language, CapabilityLanguage::Rust);
    }

    #[test]
    fn language_display() {
        assert_eq!(CapabilityLanguage::Rust.to_string(), "Rust");
        assert_eq!(CapabilityLanguage::Cpp.to_string(), "C++");
        assert_eq!(CapabilityLanguage::Python.to_string(), "Python");
        assert_eq!(CapabilityLanguage::Go.to_string(), "Go");
    }
}
