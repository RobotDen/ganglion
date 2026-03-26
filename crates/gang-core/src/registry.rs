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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum CapabilityLanguage {
    Rust,
    Cpp,
    Python,
    Go,
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
    pub name: String,
    pub latest_version: String,
    pub description: String,
    pub author: String,
    pub language: CapabilityLanguage,
    pub tags: Vec<String>,
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
    pub fn publish(&mut self, entry: RegistryEntry) -> Result<(), std::io::Error> {
        let versions = self.entries.entry(entry.name.clone()).or_default();

        // Check for duplicate version
        if versions.iter().any(|e| e.version == entry.version) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{}@{} already published", entry.name, entry.version),
            ));
        }

        versions.push(entry);
        self.persist()
    }

    /// Search for capabilities by query string.
    /// Matches against name, description, and tags.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for (name, versions) in &self.entries {
            if let Some(latest) = versions.last() {
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

    /// Get the latest version of a capability.
    pub fn get_latest(&self, name: &str) -> Option<&RegistryEntry> {
        self.entries.get(name).and_then(|v| v.last())
    }

    /// List all capabilities in the registry.
    pub fn list(&self) -> Vec<SearchResult> {
        let mut results = Vec::new();
        for (name, versions) in &self.entries {
            if let Some(latest) = versions.last() {
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

    #[test]
    fn publish_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();

        reg.publish(sample_entry("gang-capability-diagnostics", "1.0.0"))
            .unwrap();
        reg.publish(sample_entry("gang-capability-param-inspect", "1.0.0"))
            .unwrap();

        let results = reg.search("diagnostics");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "gang-capability-diagnostics");
    }

    #[test]
    fn search_by_tag() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();

        reg.publish(sample_entry("gang-capability-diagnostics", "1.0.0"))
            .unwrap();
        reg.publish(sample_entry("gang-capability-param-inspect", "1.0.0"))
            .unwrap();

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

        reg.publish(sample_entry("test-cap", "1.0.0")).unwrap();
        let result = reg.publish(sample_entry("test-cap", "1.0.0"));
        assert!(result.is_err());
    }

    #[test]
    fn multiple_versions() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();

        reg.publish(sample_entry("test-cap", "1.0.0")).unwrap();
        reg.publish(sample_entry("test-cap", "1.1.0")).unwrap();

        let versions = reg.get("test-cap").unwrap();
        assert_eq!(versions.len(), 2);

        let latest = reg.get_latest("test-cap").unwrap();
        assert_eq!(latest.version, "1.1.0");
    }

    #[test]
    fn list_all() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::open(dir.path()).unwrap();

        reg.publish(sample_entry("cap-a", "1.0.0")).unwrap();
        reg.publish(sample_entry("cap-b", "1.0.0")).unwrap();
        reg.publish(sample_entry("cap-c", "1.0.0")).unwrap();

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

        reg.publish(sample_entry("to-remove", "1.0.0")).unwrap();
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

        {
            let mut reg = Registry::open(dir.path()).unwrap();
            reg.publish(sample_entry("persistent-cap", "1.0.0"))
                .unwrap();
        }

        let reg = Registry::open(dir.path()).unwrap();
        assert_eq!(reg.count(), 1);
        let entry = reg.get_latest("persistent-cap").unwrap();
        assert_eq!(entry.version, "1.0.0");
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
