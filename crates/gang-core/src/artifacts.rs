//! Content-addressed artifact storage for Ganglion.
//!
//! Artifacts are large data objects (rosbag slices, diagnostic bundles,
//! log captures) identified by their content hash (CID). The store provides:
//!
//! - CIDv1 + Blake3 hashing for content addressing
//! - Content-addressed filesystem layout
//! - Chunking for large artifacts
//! - Block-level deduplication
//! - Configurable size cap with LRU eviction
//! - SQLite metadata index

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// CID (Content Identifier) — Blake3 hash of content.
/// Format: "bafy" prefix + hex-encoded blake3 hash (64 chars).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cid(String);

impl Cid {
    /// Compute a CID from content bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(format!("bafy{}", hash.to_hex()))
    }

    /// Compute a CID from a file on disk.
    pub fn from_file(path: &Path) -> Result<Self, std::io::Error> {
        let data = std::fs::read(path)?;
        Ok(Self::from_bytes(&data))
    }

    /// The raw string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Verify that content matches this CID.
    pub fn verify(&self, data: &[u8]) -> bool {
        Self::from_bytes(data) == *self
    }

    /// Create a CID from a raw string (e.g., from user input), validating its
    /// shape.
    ///
    /// A well-formed CID is the literal prefix `bafy` followed by exactly 64
    /// hex characters (the Blake3 digest). This validates only the string
    /// shape, not that any content with this CID exists.
    pub fn parse(s: &str) -> Result<Self, CidError> {
        const PREFIX: &str = "bafy";
        const HEX_LEN: usize = 64;

        let hex = s
            .strip_prefix(PREFIX)
            .ok_or_else(|| CidError::InvalidFormat(format!("missing `{PREFIX}` prefix: {s}")))?;
        if hex.len() != HEX_LEN {
            return Err(CidError::InvalidFormat(format!(
                "expected {HEX_LEN} hex chars after prefix, got {}",
                hex.len()
            )));
        }
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CidError::InvalidFormat(format!(
                "non-hex characters in cid: {s}"
            )));
        }
        Ok(Self(s.to_string()))
    }
}

/// Errors produced when validating a [`Cid`] string.
#[derive(Debug, thiserror::Error)]
pub enum CidError {
    /// The string does not have the expected CID shape.
    #[error("invalid cid format: {0}")]
    InvalidFormat(String),
}

impl std::fmt::Display for Cid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Metadata for a stored artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMeta {
    /// Content identifier.
    pub cid: Cid,
    /// Original filename (if known).
    pub filename: Option<String>,
    /// Size in bytes.
    pub size: u64,
    /// Number of chunks (1 if not chunked).
    pub chunk_count: u32,
    /// Peer ID of the origin.
    pub origin_peer: Option<String>,
    /// When this artifact was stored locally.
    pub stored_at: SystemTime,
    /// When this artifact was last accessed.
    pub last_accessed: SystemTime,
    /// MIME type (if known).
    pub content_type: Option<String>,
    /// Custom tags.
    pub tags: Vec<String>,
}

/// Configuration for the artifact store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactStoreConfig {
    /// Root directory for artifact storage.
    pub store_dir: PathBuf,
    /// Maximum total storage in bytes (0 = unlimited).
    pub max_size_bytes: u64,
    /// Chunk size for large artifacts (default: 1MB).
    pub chunk_size: usize,
}

impl Default for ArtifactStoreConfig {
    fn default() -> Self {
        Self {
            store_dir: PathBuf::from("/tmp/gang-artifacts"),
            max_size_bytes: 1_073_741_824, // 1 GB
            chunk_size: 1_048_576,         // 1 MB
        }
    }
}

/// Content-addressed artifact store.
///
/// Layout:
/// ```text
/// store_dir/
///   blobs/
///     ba/fy<hash>/          # First 4 chars for directory fanout
///       data                # The artifact data
///       meta.json           # Artifact metadata
///   chunks/
///     ba/fy<hash>/          # Chunk directory
///       0, 1, 2, ...        # Numbered chunk files
///   index.json              # In-memory index (persisted periodically)
/// ```
pub struct ArtifactStore {
    config: ArtifactStoreConfig,
    /// In-memory metadata index.
    index: HashMap<Cid, ArtifactMeta>,
    /// Total bytes used.
    total_bytes: u64,
    /// Number of times the index has been written to disk (observability/tests).
    persist_count: std::cell::Cell<u64>,
}

impl ArtifactStore {
    /// Open or create an artifact store at the given directory.
    pub fn open(config: ArtifactStoreConfig) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&config.store_dir)?;
        std::fs::create_dir_all(config.store_dir.join("blobs"))?;
        std::fs::create_dir_all(config.store_dir.join("chunks"))?;

        // Load existing index
        let index_path = config.store_dir.join("index.json");
        let (index, total_bytes) = if index_path.exists() {
            let data = std::fs::read_to_string(&index_path)?;
            let entries: Vec<ArtifactMeta> = serde_json::from_str(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let total: u64 = entries.iter().map(|e| e.size).sum();
            let index: HashMap<Cid, ArtifactMeta> =
                entries.into_iter().map(|e| (e.cid.clone(), e)).collect();
            (index, total)
        } else {
            (HashMap::new(), 0)
        };

        Ok(Self {
            config,
            index,
            total_bytes,
            persist_count: std::cell::Cell::new(0),
        })
    }

    /// Store an artifact. Returns the CID.
    pub fn store(
        &mut self,
        data: &[u8],
        filename: Option<&str>,
        origin_peer: Option<&str>,
        content_type: Option<&str>,
    ) -> Result<Cid, std::io::Error> {
        let cid = Cid::from_bytes(data);

        // Check if already stored (deduplication)
        if self.index.contains_key(&cid) {
            return Ok(cid);
        }

        // Evict if necessary. Each eviction deletes files but does not rewrite
        // the index; the single persist_index() at the end of store() covers
        // the whole batch (see CODE-18).
        while self.config.max_size_bytes > 0
            && self.total_bytes + data.len() as u64 > self.config.max_size_bytes
        {
            if !self.evict_lru()? {
                return Err(std::io::Error::other(
                    "artifact store full and nothing to evict",
                ));
            }
        }

        let size = data.len() as u64;

        if data.len() > self.config.chunk_size {
            // Chunked storage
            let chunk_dir = self.chunk_path(&cid);
            std::fs::create_dir_all(&chunk_dir)?;

            let chunk_count = data.len().div_ceil(self.config.chunk_size);
            for i in 0..chunk_count {
                let start = i * self.config.chunk_size;
                let end = std::cmp::min(start + self.config.chunk_size, data.len());
                std::fs::write(chunk_dir.join(i.to_string()), &data[start..end])?;
            }

            let meta = ArtifactMeta {
                cid: cid.clone(),
                filename: filename.map(String::from),
                size,
                chunk_count: chunk_count as u32,
                origin_peer: origin_peer.map(String::from),
                stored_at: SystemTime::now(),
                last_accessed: SystemTime::now(),
                content_type: content_type.map(String::from),
                tags: Vec::new(),
            };

            self.index.insert(cid.clone(), meta);
        } else {
            // Single blob storage
            let blob_dir = self.blob_path(&cid);
            std::fs::create_dir_all(&blob_dir)?;
            std::fs::write(blob_dir.join("data"), data)?;

            let meta = ArtifactMeta {
                cid: cid.clone(),
                filename: filename.map(String::from),
                size,
                chunk_count: 1,
                origin_peer: origin_peer.map(String::from),
                stored_at: SystemTime::now(),
                last_accessed: SystemTime::now(),
                content_type: content_type.map(String::from),
                tags: Vec::new(),
            };

            self.index.insert(cid.clone(), meta);
        }

        self.total_bytes += size;
        self.persist_index()?;

        Ok(cid)
    }

    /// Retrieve an artifact by CID.
    pub fn retrieve(&mut self, cid: &Cid) -> Result<Vec<u8>, std::io::Error> {
        let meta = self.index.get(cid).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("artifact {cid} not found"),
            )
        })?;

        let chunk_count = meta.chunk_count;
        let size = meta.size;

        let data = if chunk_count == 1 {
            let blob_path = self.blob_path(cid).join("data");
            std::fs::read(&blob_path)?
        } else {
            let chunk_dir = self.chunk_path(cid);
            let mut data = Vec::with_capacity(size as usize);
            for i in 0..chunk_count {
                let chunk = std::fs::read(chunk_dir.join(i.to_string()))?;
                data.extend_from_slice(&chunk);
            }
            data
        };

        // Update last_accessed after reading and persist it so LRU ordering
        // survives a restart (CODE-18).
        if let Some(meta) = self.index.get_mut(cid) {
            meta.last_accessed = SystemTime::now();
        }
        self.persist_index()?;

        Ok(data)
    }

    /// Check if an artifact exists.
    pub fn contains(&self, cid: &Cid) -> bool {
        self.index.contains_key(cid)
    }

    /// Get metadata for an artifact.
    pub fn meta(&self, cid: &Cid) -> Option<&ArtifactMeta> {
        self.index.get(cid)
    }

    /// List all artifacts.
    pub fn list(&self) -> Vec<&ArtifactMeta> {
        self.index.values().collect()
    }

    /// Remove an artifact by CID.
    pub fn remove(&mut self, cid: &Cid) -> Result<bool, std::io::Error> {
        let removed = self.remove_without_persist(cid)?;
        if removed {
            self.persist_index()?;
        }
        Ok(removed)
    }

    /// Remove an artifact's data files and index entry without rewriting the
    /// on-disk index. Callers are responsible for persisting afterwards; used
    /// by the eviction loop so a batch of evictions produces a single index
    /// write.
    fn remove_without_persist(&mut self, cid: &Cid) -> Result<bool, std::io::Error> {
        if let Some(meta) = self.index.remove(cid) {
            self.total_bytes = self.total_bytes.saturating_sub(meta.size);

            if meta.chunk_count == 1 {
                let blob_dir = self.blob_path(cid);
                if blob_dir.exists() {
                    std::fs::remove_dir_all(&blob_dir)?;
                }
            } else {
                let chunk_dir = self.chunk_path(cid);
                if chunk_dir.exists() {
                    std::fs::remove_dir_all(&chunk_dir)?;
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Total bytes used by stored artifacts.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Number of stored artifacts.
    pub fn count(&self) -> usize {
        self.index.len()
    }

    /// Evict the least recently accessed artifact.
    ///
    /// Does not persist the index; the caller (the store() eviction loop)
    /// persists once after the whole batch (CODE-18).
    fn evict_lru(&mut self) -> Result<bool, std::io::Error> {
        let lru_cid = self
            .index
            .iter()
            .min_by_key(|(_, meta)| meta.last_accessed)
            .map(|(cid, _)| cid.clone());

        if let Some(cid) = lru_cid {
            self.remove_without_persist(&cid)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn blob_path(&self, cid: &Cid) -> PathBuf {
        let s = cid.as_str();
        let prefix = &s[..4.min(s.len())];
        self.config.store_dir.join("blobs").join(prefix).join(s)
    }

    fn chunk_path(&self, cid: &Cid) -> PathBuf {
        let s = cid.as_str();
        let prefix = &s[..4.min(s.len())];
        self.config.store_dir.join("chunks").join(prefix).join(s)
    }

    fn persist_index(&self) -> Result<(), std::io::Error> {
        let entries: Vec<&ArtifactMeta> = self.index.values().collect();
        let json = serde_json::to_string_pretty(&entries).map_err(std::io::Error::other)?;
        std::fs::write(self.config.store_dir.join("index.json"), json)?;
        self.persist_count.set(self.persist_count.get() + 1);
        Ok(())
    }

    /// Number of times the index has been written to disk since this store
    /// handle was opened.
    #[cfg(test)]
    fn persist_count(&self) -> u64 {
        self.persist_count.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(dir: &Path) -> ArtifactStore {
        ArtifactStore::open(ArtifactStoreConfig {
            store_dir: dir.to_path_buf(),
            max_size_bytes: 10 * 1024 * 1024, // 10 MB
            chunk_size: 1024,                 // 1 KB for testing
        })
        .unwrap()
    }

    #[test]
    fn cid_deterministic() {
        let data = b"hello world";
        let cid1 = Cid::from_bytes(data);
        let cid2 = Cid::from_bytes(data);
        assert_eq!(cid1, cid2);
        assert!(cid1.as_str().starts_with("bafy"));
    }

    #[test]
    fn cid_different_data() {
        let cid1 = Cid::from_bytes(b"hello");
        let cid2 = Cid::from_bytes(b"world");
        assert_ne!(cid1, cid2);
    }

    #[test]
    fn cid_parse_validates_shape() {
        // A CID produced by from_bytes round-trips through parse.
        let cid = Cid::from_bytes(b"some content");
        assert!(Cid::parse(cid.as_str()).is_ok());

        // Missing prefix.
        assert!(Cid::parse(&"a".repeat(64)).is_err());
        // Wrong length.
        assert!(Cid::parse("bafyabc").is_err());
        // Non-hex characters.
        assert!(Cid::parse(&format!("bafy{}", "z".repeat(64))).is_err());
        // Correct shape.
        assert!(Cid::parse(&format!("bafy{}", "0".repeat(64))).is_ok());
    }

    #[test]
    fn cid_verify() {
        let data = b"test data";
        let cid = Cid::from_bytes(data);
        assert!(cid.verify(data));
        assert!(!cid.verify(b"wrong data"));
    }

    #[test]
    fn store_and_retrieve() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = test_store(dir.path());

        let data = b"hello world artifact";
        let cid = store
            .store(data, Some("test.bin"), Some("peer-1"), None)
            .unwrap();

        assert!(store.contains(&cid));
        assert_eq!(store.count(), 1);

        let retrieved = store.retrieve(&cid).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn deduplication() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = test_store(dir.path());

        let data = b"duplicate content";
        let cid1 = store.store(data, None, None, None).unwrap();
        let cid2 = store.store(data, None, None, None).unwrap();

        assert_eq!(cid1, cid2);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn chunked_storage() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = test_store(dir.path());

        // Create data larger than chunk_size (1KB)
        let data: Vec<u8> = (0u8..=255).cycle().take(3000).collect();
        let cid = store.store(&data, Some("large.bin"), None, None).unwrap();

        let meta = store.meta(&cid).unwrap();
        assert_eq!(meta.chunk_count, 3); // 3000 bytes / 1024 = 3 chunks

        let retrieved = store.retrieve(&cid).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn remove_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = test_store(dir.path());

        let cid = store.store(b"to be removed", None, None, None).unwrap();
        assert!(store.contains(&cid));

        let removed = store.remove(&cid).unwrap();
        assert!(removed);
        assert!(!store.contains(&cid));
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn lru_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ArtifactStore::open(ArtifactStoreConfig {
            store_dir: dir.path().to_path_buf(),
            max_size_bytes: 100, // Very small
            chunk_size: 1024,
        })
        .unwrap();

        let cid1 = store
            .store(b"first artifact 1234567890", None, None, None)
            .unwrap();
        // Access cid1 to make it recently used
        let _ = store.retrieve(&cid1).unwrap();

        // This should trigger eviction of cid1 if store is full
        let data2: Vec<u8> = vec![0u8; 80];
        let cid2 = store.store(&data2, None, None, None).unwrap();

        // cid1 should have been evicted
        assert!(!store.contains(&cid1));
        assert!(store.contains(&cid2));
    }

    #[test]
    fn eviction_rewrites_index_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ArtifactStore::open(ArtifactStoreConfig {
            store_dir: dir.path().to_path_buf(),
            max_size_bytes: 100,
            chunk_size: 1024,
        })
        .unwrap();

        // Fill the store with several small artifacts (no eviction yet).
        store.store(&vec![1u8; 30], None, None, None).unwrap();
        store.store(&vec![2u8; 30], None, None, None).unwrap();
        store.store(&vec![3u8; 30], None, None, None).unwrap();
        assert_eq!(store.count(), 3);

        // Now store a large artifact that forces evicting multiple entries.
        let before = store.persist_count();
        store.store(&vec![9u8; 90], None, None, None).unwrap();
        let after = store.persist_count();

        // The whole store() call (multiple evictions + insert) rewrites the
        // index exactly once.
        assert_eq!(after - before, 1, "expected a single index write");
        // Some older entries were evicted to make room.
        assert!(store.count() < 4);
    }

    #[test]
    fn last_accessed_persists_across_restart() {
        let dir = tempfile::tempdir().unwrap();

        let (cid_a, cid_b) = {
            let mut store = ArtifactStore::open(ArtifactStoreConfig {
                store_dir: dir.path().to_path_buf(),
                max_size_bytes: 200,
                chunk_size: 1024,
            })
            .unwrap();
            let a = store.store(&vec![1u8; 50], None, None, None).unwrap();
            let b = store.store(&vec![2u8; 50], None, None, None).unwrap();
            // Access A so it becomes most-recently-used; this must persist.
            let _ = store.retrieve(&a).unwrap();
            (a, b)
        };

        // Reopen (fresh handle, index loaded from disk).
        let mut store = ArtifactStore::open(ArtifactStoreConfig {
            store_dir: dir.path().to_path_buf(),
            max_size_bytes: 200,
            chunk_size: 1024,
        })
        .unwrap();

        // Store an artifact forcing a single eviction; B (older access) should
        // go, A should survive — proving last_accessed was persisted before
        // restart.
        store.store(&vec![3u8; 120], None, None, None).unwrap();
        assert!(store.contains(&cid_a), "A was accessed last and should survive");
        assert!(!store.contains(&cid_b), "B was least-recently-used and should be evicted");
    }

    #[test]
    fn persist_and_reload() {
        let dir = tempfile::tempdir().unwrap();

        let cid = {
            let mut store = test_store(dir.path());
            store
                .store(b"persistent data", Some("file.txt"), None, None)
                .unwrap()
        };

        // Reopen the store
        let store = test_store(dir.path());
        assert!(store.contains(&cid));
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn list_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = test_store(dir.path());

        store
            .store(b"artifact 1", Some("a.bin"), None, None)
            .unwrap();
        store
            .store(b"artifact 2", Some("b.bin"), None, None)
            .unwrap();
        store
            .store(b"artifact 3", Some("c.bin"), None, None)
            .unwrap();

        let list = store.list();
        assert_eq!(list.len(), 3);
    }
}
