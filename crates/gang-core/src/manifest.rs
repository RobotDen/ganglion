use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::capability::CapabilityGroup;
use crate::error::ManifestError;
use crate::identity::{Keypair, PeerId};
use crate::registry::CapabilityLanguage;

/// Manifest schema version.
pub const MANIFEST_SCHEMA_VERSION: &str = "2.0";

/// Signed manifest that accompanies every WASM component.
/// §3.6: Every WASM component ships with a manifest containing component metadata,
/// declared capabilities, author identity, and a signature over the component bytes
/// and manifest by the author's private key.
///
/// Schema v2.0 (Ganglion 0.4): adds authoring language, description, tags,
/// minimum Ganglion version, and schema version field. v1.x manifests load
/// with a deprecation warning via default fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentManifest {
    /// Manifest schema version ("2.0" for v0.4+).
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Capability name.
    pub name: String,
    /// Capability version.
    pub version: String,
    /// Capability groups the component declares.
    pub declared_capabilities: Vec<CapabilityGroup>,
    /// Peer ID of the manifest author.
    pub author_peer_id: PeerId,
    /// Blake3 hash of the component .wasm bytes.
    pub component_hash: String,
    /// Optional resource limits.
    #[serde(default)]
    pub limits: ResourceLimits,
    /// Language the capability was authored in (v2.0).
    #[serde(default)]
    pub language: CapabilityLanguage,
    /// Short description (v2.0).
    #[serde(default)]
    pub description: String,
    /// Tags for registry discoverability (v2.0).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Minimum Ganglion version required (v2.0).
    #[serde(default)]
    pub min_ganglion_version: Option<String>,
}

fn default_schema_version() -> String {
    "1.0".to_string()
}

/// Optional resource limits for a WASM component.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes (0 = use default).
    #[serde(default)]
    pub max_memory_bytes: u64,
    /// CPU fuel budget (0 = unlimited).
    #[serde(default)]
    pub cpu_fuel: u64,
    /// Wall-clock deadline in seconds (0 = no deadline).
    #[serde(default)]
    pub wall_clock_secs: u64,
}

/// A manifest with its signature attached — the wire/storage format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedManifest {
    /// CBOR-encoded [`ComponentManifest`] bytes (the signed payload).
    pub manifest_cbor: Vec<u8>,
    /// Ed25519 signature over `manifest_cbor`.
    pub signature: Vec<u8>,
    /// The author's Ed25519 public key bytes.
    pub author_public_key: Vec<u8>,
}

impl SignedManifest {
    /// Sign a manifest with a keypair. The signature covers the CBOR-encoded
    /// manifest bytes concatenated with the component hash.
    pub fn sign(manifest: &ComponentManifest, keypair: &Keypair) -> Result<Self, ManifestError> {
        let mut manifest_cbor = Vec::new();
        ciborium::into_writer(manifest, &mut manifest_cbor)
            .map_err(|e| ManifestError::DecodeFailed(e.to_string()))?;

        let sig = keypair.sign(&manifest_cbor);

        Ok(Self {
            manifest_cbor,
            signature: sig.to_bytes().to_vec(),
            author_public_key: keypair.public_key().to_bytes().to_vec(),
        })
    }

    /// Verify the signature and decode the manifest.
    pub fn verify_and_decode(&self) -> Result<ComponentManifest, ManifestError> {
        // Reconstruct public key
        let pub_key_bytes: [u8; 32] = self
            .author_public_key
            .as_slice()
            .try_into()
            .map_err(|_| ManifestError::InvalidSignature)?;
        let public_key = VerifyingKey::from_bytes(&pub_key_bytes)
            .map_err(|_| ManifestError::InvalidSignature)?;

        // Reconstruct signature
        let sig_bytes: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| ManifestError::InvalidSignature)?;
        let signature = Signature::from_bytes(&sig_bytes);

        // Verify
        if !Keypair::verify(&public_key, &self.manifest_cbor, &signature) {
            return Err(ManifestError::InvalidSignature);
        }

        // Decode
        let manifest: ComponentManifest = ciborium::from_reader(&self.manifest_cbor[..])
            .map_err(|e| ManifestError::DecodeFailed(e.to_string()))?;

        // Verify peer ID matches the public key
        let expected_peer_id = PeerId::from_public_key(&public_key);
        if manifest.author_peer_id != expected_peer_id {
            return Err(ManifestError::InvalidSignature);
        }

        Ok(manifest)
    }

    /// Verify the manifest and also check that the component hash matches
    /// the provided component bytes.
    pub fn verify_with_component(
        &self,
        component_bytes: &[u8],
    ) -> Result<ComponentManifest, ManifestError> {
        let manifest = self.verify_and_decode()?;

        let actual_hash = blake3::hash(component_bytes).to_hex().to_string();
        if actual_hash != manifest.component_hash {
            return Err(ManifestError::HashMismatch {
                expected: manifest.component_hash.clone(),
                actual: actual_hash,
            });
        }

        Ok(manifest)
    }

    /// Encode to CBOR bytes for wire transfer or storage.
    pub fn to_cbor(&self) -> Result<Vec<u8>, ManifestError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| ManifestError::DecodeFailed(e.to_string()))?;
        Ok(buf)
    }

    /// Decode from CBOR bytes.
    pub fn from_cbor(data: &[u8]) -> Result<Self, ManifestError> {
        ciborium::from_reader(data).map_err(|e| ManifestError::DecodeFailed(e.to_string()))
    }
}

/// Trust store: a list of peer IDs whose signatures are accepted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    /// The peers whose signatures are accepted.
    pub trusted_peers: Vec<TrustedPeer>,
}

/// A single trusted peer entry: an identity plus its public key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedPeer {
    /// The trusted peer's ID.
    pub peer_id: PeerId,
    /// Human-readable name for the peer.
    pub name: String,
    /// The peer's Ed25519 public key bytes.
    pub public_key: Vec<u8>,
}

impl TrustStore {
    /// Load the trust store from `path`, returning an empty store if absent.
    pub fn load(path: &std::path::Path) -> Result<Self, std::io::Error> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        serde_json::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Persist the trust store to `path`, creating parent directories.
    pub fn save(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, json)
    }

    /// Whether the given peer is present in the trust store.
    pub fn is_trusted(&self, peer_id: &PeerId) -> bool {
        self.trusted_peers.iter().any(|p| &p.peer_id == peer_id)
    }

    /// The stored public key bytes for a trusted peer, if present.
    pub fn get_public_key(&self, peer_id: &PeerId) -> Option<&[u8]> {
        self.trusted_peers
            .iter()
            .find(|p| &p.peer_id == peer_id)
            .map(|p| p.public_key.as_slice())
    }

    /// Add a peer to the trust store (no-op if already present).
    pub fn add(&mut self, peer: TrustedPeer) {
        if !self.is_trusted(&peer.peer_id) {
            self.trusted_peers.push(peer);
        }
    }

    /// Remove a peer from the trust store.
    pub fn remove(&mut self, peer_id: &PeerId) {
        self.trusted_peers.retain(|p| &p.peer_id != peer_id);
    }

    /// Find the index of a trusted peer entry, if it exists.
    pub fn index_of(&self, peer_id: &PeerId) -> Option<usize> {
        self.trusted_peers
            .iter()
            .position(|p| &p.peer_id == peer_id)
    }

    /// Find a trusted peer by its recorded human-readable name.
    ///
    /// Used by SSH-style host-key verification: a robot *name* that was
    /// previously bound to one identity but now presents another is the
    /// "REMOTE HOST IDENTIFICATION HAS CHANGED" case (the peer id itself is
    /// key-derived, so a changed key always means a changed peer id).
    pub fn find_by_name(&self, name: &str) -> Option<&TrustedPeer> {
        self.trusted_peers.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_manifest(keypair: &Keypair, component_bytes: &[u8]) -> ComponentManifest {
        ComponentManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.into(),
            name: "test-capability".into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![],
            author_peer_id: keypair.peer_id(),
            component_hash: blake3::hash(component_bytes).to_hex().to_string(),
            limits: ResourceLimits::default(),
            language: CapabilityLanguage::Rust,
            description: "A test capability".into(),
            tags: vec!["test".into()],
            min_ganglion_version: Some("0.4.0".into()),
        }
    }

    #[test]
    fn sign_and_verify_manifest() {
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let manifest = make_test_manifest(&keypair, component);

        let signed = SignedManifest::sign(&manifest, &keypair).unwrap();
        let decoded = signed.verify_and_decode().unwrap();

        assert_eq!(decoded.name, "test-capability");
        assert_eq!(decoded.author_peer_id, keypair.peer_id());
    }

    #[test]
    fn verify_with_component_succeeds() {
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let manifest = make_test_manifest(&keypair, component);

        let signed = SignedManifest::sign(&manifest, &keypair).unwrap();
        let decoded = signed.verify_with_component(component).unwrap();
        assert_eq!(decoded.name, "test-capability");
    }

    #[test]
    fn verify_with_wrong_component_fails() {
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let manifest = make_test_manifest(&keypair, component);

        let signed = SignedManifest::sign(&manifest, &keypair).unwrap();
        let result = signed.verify_with_component(b"different bytes");
        assert!(matches!(result, Err(ManifestError::HashMismatch { .. })));
    }

    #[test]
    fn tampered_manifest_fails() {
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let manifest = make_test_manifest(&keypair, component);

        let mut signed = SignedManifest::sign(&manifest, &keypair).unwrap();
        // Tamper with the manifest bytes
        if let Some(byte) = signed.manifest_cbor.last_mut() {
            *byte ^= 0xFF;
        }
        assert!(signed.verify_and_decode().is_err());
    }

    #[test]
    fn wrong_signer_fails_trust_check() {
        let keypair = Keypair::generate();
        let other = Keypair::generate();

        let mut store = TrustStore::default();
        store.add(TrustedPeer {
            peer_id: other.peer_id(),
            name: "other".into(),
            public_key: other.public_key().to_bytes().to_vec(),
        });

        // keypair signed it, but only other is trusted
        assert!(!store.is_trusted(&keypair.peer_id()));
        assert!(store.is_trusted(&other.peer_id()));
    }

    #[test]
    fn schema_version_defaults_to_1() {
        // Simulate a v1.x manifest missing the new fields
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let hash = blake3::hash(component).to_hex().to_string();

        // Build a minimal JSON that only has v1 fields
        let json = serde_json::json!({
            "name": "old-cap",
            "version": "0.1.0",
            "declared_capabilities": [],
            "author_peer_id": keypair.peer_id().as_str(),
            "component_hash": hash,
        });

        let manifest: ComponentManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.schema_version, "1.0"); // default
        assert_eq!(manifest.language, CapabilityLanguage::Other); // default
        assert!(manifest.description.is_empty());
        assert!(manifest.tags.is_empty());
        assert!(manifest.min_ganglion_version.is_none());
    }

    #[test]
    fn v2_manifest_fields() {
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let manifest = make_test_manifest(&keypair, component);

        assert_eq!(manifest.schema_version, "2.0");
        assert_eq!(manifest.language, CapabilityLanguage::Rust);
        assert!(!manifest.description.is_empty());
        assert!(!manifest.tags.is_empty());
    }

    #[test]
    fn signed_manifest_cbor_roundtrip() {
        let keypair = Keypair::generate();
        let component = b"fake wasm bytes";
        let manifest = make_test_manifest(&keypair, component);

        let signed = SignedManifest::sign(&manifest, &keypair).unwrap();
        let cbor = signed.to_cbor().unwrap();
        let decoded = SignedManifest::from_cbor(&cbor).unwrap();
        let manifest2 = decoded.verify_and_decode().unwrap();
        assert_eq!(manifest2.name, "test-capability");
    }
}
