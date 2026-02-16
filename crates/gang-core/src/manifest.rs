use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::capability::CapabilityGroup;
use crate::error::ManifestError;
use crate::identity::{Keypair, PeerId};

/// Signed manifest that accompanies every WASM component.
/// §3.6: Every WASM component ships with a manifest containing component metadata,
/// declared capabilities, author identity, and a signature over the component bytes
/// and manifest by the author's private key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentManifest {
    pub name: String,
    pub version: String,
    pub declared_capabilities: Vec<CapabilityGroup>,
    pub author_peer_id: PeerId,
    /// Blake3 hash of the component .wasm bytes.
    pub component_hash: String,
    /// Optional resource limits.
    #[serde(default)]
    pub limits: ResourceLimits,
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
    pub manifest_cbor: Vec<u8>,
    pub signature: Vec<u8>,
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
        let pub_key_bytes: [u8; 32] = self.author_public_key
            .as_slice()
            .try_into()
            .map_err(|_| ManifestError::InvalidSignature)?;
        let public_key = VerifyingKey::from_bytes(&pub_key_bytes)
            .map_err(|_| ManifestError::InvalidSignature)?;

        // Reconstruct signature
        let sig_bytes: [u8; 64] = self.signature
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
        ciborium::from_reader(data)
            .map_err(|e| ManifestError::DecodeFailed(e.to_string()))
    }
}

/// Trust store: a list of peer IDs whose signatures are accepted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    pub trusted_peers: Vec<TrustedPeer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub peer_id: PeerId,
    pub name: String,
    pub public_key: Vec<u8>,
}

impl TrustStore {
    pub fn load(path: &std::path::Path) -> Result<Self, std::io::Error> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        serde_json::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        std::fs::write(path, json)
    }

    pub fn is_trusted(&self, peer_id: &PeerId) -> bool {
        self.trusted_peers.iter().any(|p| &p.peer_id == peer_id)
    }

    pub fn get_public_key(&self, peer_id: &PeerId) -> Option<&[u8]> {
        self.trusted_peers
            .iter()
            .find(|p| &p.peer_id == peer_id)
            .map(|p| p.public_key.as_slice())
    }

    pub fn add(&mut self, peer: TrustedPeer) {
        if !self.is_trusted(&peer.peer_id) {
            self.trusted_peers.push(peer);
        }
    }

    pub fn remove(&mut self, peer_id: &PeerId) {
        self.trusted_peers.retain(|p| &p.peer_id != peer_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_manifest(keypair: &Keypair, component_bytes: &[u8]) -> ComponentManifest {
        ComponentManifest {
            name: "test-capability".into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![],
            author_peer_id: keypair.peer_id(),
            component_hash: blake3::hash(component_bytes).to_hex().to_string(),
            limits: ResourceLimits::default(),
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
