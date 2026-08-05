use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

/// Errors produced when validating identity material.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The supplied string is not a well-formed peer ID.
    #[error("invalid peer id: {0}")]
    InvalidPeerId(String),
}

/// A Ganglion peer identity derived from an Ed25519 keypair.
/// The peer ID is the canonical identifier in logs, capability policies, and audit records.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(String);

impl PeerId {
    /// Derive a peer ID from a public key (hex-encoded public key bytes).
    pub fn from_public_key(key: &VerifyingKey) -> Self {
        Self::from_ed25519_bytes(key.as_bytes())
    }

    /// Derive a peer ID from the raw 32-byte Ed25519 public key.
    ///
    /// This is the single canonical derivation (SEC-03): every layer — the
    /// core identity, the trust store, signed manifests, and the libp2p
    /// transport adapter — must produce a peer ID from these same raw key
    /// bytes so that policy `peer_rules` keyed on a real identity match the
    /// identity observed on the wire.
    pub fn from_ed25519_bytes(key_bytes: &[u8]) -> Self {
        let hash = blake3::hash(key_bytes);
        Self(format!("12D3-{}", hex::encode(&hash.as_bytes()[..16])))
    }

    /// Construct a peer ID from a string (e.g., parsed from CLI input or config).
    ///
    /// This validates the string shape (`12D3-` prefix followed by 32 lowercase
    /// hex characters) and panics on malformed input. Prefer [`PeerId::parse`]
    /// when the input is untrusted and you want to handle errors gracefully.
    pub fn new(s: &str) -> Self {
        Self::parse(s).expect("invalid peer id")
    }

    /// Validate and construct a peer ID from a string.
    ///
    /// A well-formed peer ID is the literal prefix `12D3-` followed by exactly
    /// 32 hex characters (the truncated Blake3 digest of the public key). This
    /// validates only the string *shape*; it does not attempt to re-derive the
    /// ID from a key.
    pub fn parse(s: &str) -> Result<Self, IdentityError> {
        const PREFIX: &str = "12D3-";
        const HEX_LEN: usize = 32;

        let hex = s.strip_prefix(PREFIX).ok_or_else(|| {
            IdentityError::InvalidPeerId(format!("missing `{PREFIX}` prefix: {s}"))
        })?;
        if hex.len() != HEX_LEN {
            return Err(IdentityError::InvalidPeerId(format!(
                "expected {HEX_LEN} hex chars after prefix, got {}",
                hex.len()
            )));
        }
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(IdentityError::InvalidPeerId(format!(
                "non-hex characters in peer id: {s}"
            )));
        }
        Ok(Self(s.to_string()))
    }

    /// The peer ID as a string slice.
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
    ///
    /// If the file permissions are looser than `0600` the mode is repaired
    /// (tightened to owner read/write only) before the key is used. A warning
    /// is logged when a repair is performed.
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        #[cfg(unix)]
        Self::repair_permissions(path)?;

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

    /// If the secret key file is readable/writable by group or others, tighten
    /// its mode to `0600`. Best-effort: logs a warning if the mode cannot be
    /// adjusted rather than failing the load.
    #[cfg(unix)]
    fn repair_permissions(path: &Path) -> Result<(), std::io::Error> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = std::fs::metadata(path)?;
        let mode = metadata.permissions().mode();
        // Only the lower permission bits matter here.
        if mode & 0o077 != 0 {
            tracing::warn!(
                path = %path.display(),
                mode = format!("{:o}", mode & 0o7777),
                "identity key file has loose permissions; repairing to 0600"
            );
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            if let Err(e) = std::fs::set_permissions(path, perms) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to repair identity key file permissions"
                );
            }
        }
        Ok(())
    }

    /// Save the keypair to a file (raw 32-byte secret key).
    ///
    /// On Unix the secret key file is created with mode `0600` and its parent
    /// directory (if created here) with mode `0700`, so the key material is
    /// only accessible to the owning user.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Tighten the directory to 0700 (best-effort; only if it exists).
                if let Ok(meta) = std::fs::metadata(parent) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o700);
                    let _ = std::fs::set_permissions(parent, perms);
                }
            }
        }

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;

            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            // If the file pre-existed with looser perms, enforce 0600.
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata()?.permissions();
                perms.set_mode(0o600);
                let _ = file.set_permissions(perms);
            }
            file.write_all(&self.signing_key.to_bytes())
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, self.signing_key.to_bytes())
        }
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

    /// The peer ID derived from this keypair's public key.
    pub fn peer_id(&self) -> PeerId {
        PeerId::from_public_key(&self.signing_key.verifying_key())
    }

    /// This keypair's Ed25519 public (verifying) key.
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Sign a message with this keypair's private key.
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Verify a signature over `message` against `public_key`.
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

/// A registry entry binding a name to a peer's ID, role, and relay addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    /// The peer's identifier.
    pub peer_id: PeerId,
    /// The peer's network role.
    pub role: Role,
    /// Known relay multiaddresses for reaching this peer.
    #[serde(default)]
    pub relay_addrs: Vec<String>,
    /// The peer's dialable libp2p peer id (base58 `12D3KooW…`), when known.
    ///
    /// The gang `peer_id` above identifies the peer in trust stores and
    /// policies, but only the libp2p form can appear in a `/p2p/` multiaddr
    /// component — remote dispatch requires it. Registry files written before
    /// this field existed load with `None`; re-register the peer with the
    /// libp2p id (printed by `gang agent`/`gang relay`) to populate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libp2p_id: Option<String>,
}

impl PeerRegistry {
    /// Load the registry from `path`, returning an empty registry if absent.
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        serde_json::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Persist the registry to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, json)
    }

    /// Register (or replace) a named peer entry.
    pub fn register(&mut self, name: String, entry: PeerEntry) {
        self.entries.insert(name, entry);
    }

    /// Look up an entry by its human-readable name.
    pub fn lookup(&self, name: &str) -> Option<&PeerEntry> {
        self.entries.get(name)
    }

    /// Look up the name and entry for a given peer ID.
    pub fn lookup_by_peer_id(&self, peer_id: &PeerId) -> Option<(&str, &PeerEntry)> {
        self.entries
            .iter()
            .find(|(_, entry)| &entry.peer_id == peer_id)
            .map(|(name, entry)| (name.as_str(), entry))
    }

    /// Remove a named entry, returning it if it existed.
    pub fn remove(&mut self, name: &str) -> Option<PeerEntry> {
        self.entries.remove(name)
    }

    /// Iterate over all `(name, entry)` pairs.
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
///
/// Honors the `GANG_KEY_PATH` environment variable when set (used for relay
/// identity persistence and testing), falling back to
/// `~/.gang/identity.key`.
pub fn default_key_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GANG_KEY_PATH")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
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

    #[cfg(unix)]
    #[test]
    fn saved_key_file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.key");

        let kp = Keypair::generate();
        kp.save(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "expected 0600, got {:o}", mode & 0o777);
    }

    #[cfg(unix)]
    #[test]
    fn load_repairs_loose_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.key");

        let kp = Keypair::generate();
        kp.save(&path).unwrap();

        // Loosen permissions to world-readable/writable.
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o666);
        std::fs::set_permissions(&path, perms).unwrap();

        // Load should repair the mode.
        let _ = Keypair::load(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "expected group/other bits cleared, got {:o}",
            mode & 0o777
        );
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
            libp2p_id: Some("12D3KooWExampleDialableId".into()),
        };

        registry.register("robot-42".into(), entry.clone());
        assert!(registry.lookup("robot-42").is_some());
        assert_eq!(registry.lookup("robot-42").unwrap().peer_id, kp.peer_id());

        // Save and reload
        registry.save(&path).unwrap();
        let loaded = PeerRegistry::load(&path).unwrap();
        assert!(loaded.lookup("robot-42").is_some());
        assert_eq!(
            loaded.lookup("robot-42").unwrap().libp2p_id.as_deref(),
            Some("12D3KooWExampleDialableId")
        );

        // Lookup by peer ID
        let (name, _) = loaded.lookup_by_peer_id(&kp.peer_id()).unwrap();
        assert_eq!(name, "robot-42");
    }

    #[test]
    fn peer_registry_loads_pre_libp2p_id_files() {
        // Registry files written before the `libp2p_id` field existed must
        // still load (back-compat), with `libp2p_id: None`.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("peers.json");
        std::fs::write(
            &path,
            r#"{"entries":{"old-bot":{"peer_id":"12D3-0123456789abcdef0123456789abcdef","role":"robot_agent","relay_addrs":["/ip4/1.2.3.4/tcp/4001"]}}}"#,
        )
        .unwrap();

        let loaded = PeerRegistry::load(&path).unwrap();
        let entry = loaded.lookup("old-bot").expect("legacy entry loads");
        assert!(entry.libp2p_id.is_none());
        assert_eq!(entry.relay_addrs.len(), 1);
    }

    #[test]
    fn peer_id_parse_validates_shape() {
        // A well-formed derived peer ID round-trips through parse.
        let kp = Keypair::generate();
        let good = kp.peer_id();
        assert!(PeerId::parse(good.as_str()).is_ok());

        // Missing prefix.
        assert!(PeerId::parse("deadbeefdeadbeefdeadbeefdeadbeef").is_err());
        // Wrong length.
        assert!(PeerId::parse("12D3-abc").is_err());
        // Non-hex characters.
        assert!(PeerId::parse("12D3-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
        // Correct shape.
        assert!(PeerId::parse("12D3-0123456789abcdef0123456789abcdef").is_ok());
    }

    #[test]
    fn default_key_path_honors_env() {
        // Save/restore to avoid cross-test contamination.
        let prev = std::env::var_os("GANG_KEY_PATH");

        unsafe {
            std::env::set_var("GANG_KEY_PATH", "/custom/relay/identity.key");
        }
        assert_eq!(
            default_key_path(),
            PathBuf::from("/custom/relay/identity.key")
        );

        unsafe {
            std::env::remove_var("GANG_KEY_PATH");
        }
        assert!(default_key_path().ends_with("identity.key"));

        // Restore prior value if any.
        if let Some(v) = prev {
            unsafe {
                std::env::set_var("GANG_KEY_PATH", v);
            }
        }
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::RobotAgent.to_string(), "robot-agent");
        assert_eq!(Role::Operator.to_string(), "operator");
        assert_eq!(Role::Relay.to_string(), "relay");
    }
}
