use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

/// A Ganglion peer identity derived from an Ed25519 keypair.
/// The peer ID is the canonical identifier in logs, capability policies, and audit records.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(String);

impl PeerId {
    /// Derive a peer ID from a public key (hex-encoded public key bytes).
    pub fn from_public_key(key: &VerifyingKey) -> Self {
        let hash = blake3::hash(key.as_bytes());
        Self(format!("12D3-{}", hex::encode(&hash.as_bytes()[..16])))
    }

    /// Construct a peer ID from a string (e.g., parsed from CLI input or config).
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check whether `prefix` is a prefix of this peer ID.
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.0.starts_with(prefix)
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The keypair backing a peer identity.
pub struct Keypair {
    signing_key: SigningKey,
}

impl Keypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut OsRng),
        }
    }

    /// Load a keypair from a file (raw 32-byte secret key).
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let bytes = std::fs::read(path)?;
        if bytes.len() != 32 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("expected 32-byte key, got {} bytes", bytes.len()),
            ));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        Ok(Self {
            signing_key: SigningKey::from_bytes(&key_bytes),
        })
    }

    /// Save the keypair to a file (raw 32-byte secret key).
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.signing_key.to_bytes())
    }

    /// Load or generate a keypair at the given path.
    pub fn load_or_generate(path: &Path) -> Result<Self, std::io::Error> {
        if path.exists() {
            Self::load(path)
        } else {
            let kp = Self::generate();
            kp.save(path)?;
            Ok(kp)
        }
    }

    pub fn peer_id(&self) -> PeerId {
        PeerId::from_public_key(&self.signing_key.verifying_key())
    }

    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    pub fn verify(public_key: &VerifyingKey, message: &[u8], signature: &Signature) -> bool {
        public_key.verify(message, signature).is_ok()
    }
}

/// Three first-class roles in the Ganglion network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Native Ganglion process on a deployed robot. Dials out to relays,
    /// hosts the WASM runtime and Layer 3 brokers.
    RobotAgent,
    /// Human or automated system acting on a fleet. Interacts via the
    /// `gang` CLI or operator libraries.
    Operator,
    /// Circuit relay enabling connections when neither party can accept inbound.
    Relay,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::RobotAgent => write!(f, "robot-agent"),
            Role::Operator => write!(f, "operator"),
            Role::Relay => write!(f, "relay"),
        }
    }
}

/// Local registry mapping human-readable names to peer IDs.
/// Human-readable names are bindings over peer IDs — they are not authoritative.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerRegistry {
    entries: HashMap<String, PeerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    pub peer_id: PeerId,
    pub role: Role,
    #[serde(default)]
    pub relay_addrs: Vec<String>,
}

impl PeerRegistry {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        serde_json::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, json)
    }

    pub fn register(&mut self, name: String, entry: PeerEntry) {
        self.entries.insert(name, entry);
    }

    pub fn lookup(&self, name: &str) -> Option<&PeerEntry> {
        self.entries.get(name)
    }

    pub fn lookup_by_peer_id(&self, peer_id: &PeerId) -> Option<(&str, &PeerEntry)> {
        self.entries
            .iter()
            .find(|(_, entry)| &entry.peer_id == peer_id)
            .map(|(name, entry)| (name.as_str(), entry))
    }

    pub fn remove(&mut self, name: &str) -> Option<PeerEntry> {
        self.entries.remove(name)
    }

    pub fn list(&self) -> impl Iterator<Item = (&str, &PeerEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Find peers whose peer ID starts with the given prefix.
    /// Returns all matches — callers should reject ambiguous prefixes.
    pub fn lookup_by_prefix(&self, prefix: &str) -> Vec<(&str, &PeerEntry)> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.peer_id.starts_with(prefix))
            .map(|(name, entry)| (name.as_str(), entry))
            .collect()
    }
}

/// Default path for the gang config directory.
pub fn default_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gang")
}

/// Default path for the identity key file.
pub fn default_key_path() -> PathBuf {
    default_config_dir().join("identity.key")
}

/// Default path for the peer registry.
pub fn default_registry_path() -> PathBuf {
    default_config_dir().join("peers.json")
}

/// Default path for the trust store.
pub fn default_trust_store_path() -> PathBuf {
    default_config_dir().join("trusted_peers.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn keypair_generate_and_derive_peer_id() {
        let kp = Keypair::generate();
        let peer_id = kp.peer_id();
        assert!(peer_id.as_str().starts_with("12D3-"));
        assert_eq!(peer_id.as_str().len(), 5 + 32); // "12D3-" + 32 hex chars
    }

    #[test]
    fn keypair_persist_and_reload() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.key");

        let kp1 = Keypair::generate();
        kp1.save(&path).unwrap();

        let kp2 = Keypair::load(&path).unwrap();
        assert_eq!(kp1.peer_id(), kp2.peer_id());
    }

    #[test]
    fn keypair_load_or_generate() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.key");

        // First call generates
        let kp1 = Keypair::load_or_generate(&path).unwrap();
        assert!(path.exists());

        // Second call loads the same key
        let kp2 = Keypair::load_or_generate(&path).unwrap();
        assert_eq!(kp1.peer_id(), kp2.peer_id());
    }

    #[test]
    fn sign_and_verify() {
        let kp = Keypair::generate();
        let message = b"hello ganglion";
        let sig = kp.sign(message);
        assert!(Keypair::verify(&kp.public_key(), message, &sig));

        // Tampered message fails
        assert!(!Keypair::verify(&kp.public_key(), b"tampered", &sig));
    }

    #[test]
    fn peer_registry_crud() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("peers.json");

        let mut registry = PeerRegistry::default();
        let kp = Keypair::generate();
        let entry = PeerEntry {
            peer_id: kp.peer_id(),
            role: Role::RobotAgent,
            relay_addrs: vec!["/ip4/1.2.3.4/tcp/4001".into()],
        };

        registry.register("robot-42".into(), entry.clone());
        assert!(registry.lookup("robot-42").is_some());
        assert_eq!(registry.lookup("robot-42").unwrap().peer_id, kp.peer_id());

        // Save and reload
        registry.save(&path).unwrap();
        let loaded = PeerRegistry::load(&path).unwrap();
        assert!(loaded.lookup("robot-42").is_some());

        // Lookup by peer ID
        let (name, _) = loaded.lookup_by_peer_id(&kp.peer_id()).unwrap();
        assert_eq!(name, "robot-42");
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::RobotAgent.to_string(), "robot-agent");
        assert_eq!(Role::Operator.to_string(), "operator");
        assert_eq!(Role::Relay.to_string(), "relay");
    }
}
