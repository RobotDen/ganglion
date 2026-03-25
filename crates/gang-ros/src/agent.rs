use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use gang_core::audit::{AuditLog, AuditRecord, CapabilityIoStats, ExitStatus};
use gang_core::broker::CapabilityBroker;
use gang_core::capability::{CapabilityGroup, InstalledCapability};
use gang_core::error::{CapabilityError, ManifestError};
use gang_core::identity::{Keypair, PeerId};
use gang_core::manifest::{SignedManifest, TrustStore};
use gang_core::policy::Policy;

use crate::diagnostics::DiagnosticsBroker;
use crate::filesystem::{FsBroker, FsRule};
use crate::logs::LogStreamBroker;

/// The robot agent — the central process running on a deployed robot.
/// Manages installed capabilities, brokers, policy enforcement, and audit logging.
pub struct RobotAgent {
    /// This robot's identity.
    keypair: Keypair,
    peer_id: PeerId,

    /// Installed capabilities indexed by name.
    capabilities: Arc<RwLock<HashMap<String, InstalledCapability>>>,

    /// Layer 3 brokers.
    diagnostics_broker: DiagnosticsBroker,
    log_broker: LogStreamBroker,
    fs_broker: FsBroker,

    /// Policy engine.
    policy: Policy,

    /// Trust store for verifying capability signatures.
    trust_store: TrustStore,

    /// Audit log.
    audit_log: AuditLog,

    /// Directory for storing installed capabilities.
    capabilities_dir: PathBuf,
}

/// Configuration for the robot agent.
pub struct AgentConfig {
    pub key_path: PathBuf,
    pub policy_path: Option<PathBuf>,
    pub trust_store_path: PathBuf,
    pub capabilities_dir: PathBuf,
    pub audit_log_path: PathBuf,
    pub audit_max_size_bytes: u64,
    pub fs_allowed_patterns: Vec<FsRule>,
    pub log_allowed_sources: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            key_path: gang_core::identity::default_key_path(),
            policy_path: None,
            trust_store_path: gang_core::identity::default_trust_store_path(),
            capabilities_dir: PathBuf::from("/var/lib/gang/capabilities"),
            audit_log_path: PathBuf::from("/var/lib/gang/audit.log"),
            audit_max_size_bytes: 50 * 1024 * 1024, // 50 MB
            fs_allowed_patterns: vec![FsRule {
                pattern: "/tmp/gang/**".into(),
                read: true,
                write: true,
            }],
            log_allowed_sources: vec!["**".into()],
        }
    }
}

impl RobotAgent {
    /// Create a new robot agent from configuration.
    pub fn new(config: AgentConfig) -> anyhow::Result<Self> {
        let keypair = Keypair::load_or_generate(&config.key_path)?;
        let peer_id = keypair.peer_id();

        info!(peer_id = %peer_id, "Robot agent initializing");

        // Load policy
        let policy = if let Some(ref path) = config.policy_path {
            Policy::load(path).unwrap_or_else(|e| {
                warn!("Failed to load policy from {}: {e}, using permissive", path.display());
                Policy::permissive()
            })
        } else {
            info!("No policy file configured, using permissive policy");
            Policy::permissive()
        };

        // Load trust store
        let trust_store = TrustStore::load(&config.trust_store_path).unwrap_or_else(|e| {
            warn!("Failed to load trust store: {e}, starting empty");
            TrustStore::default()
        });

        // Create capabilities directory
        std::fs::create_dir_all(&config.capabilities_dir)?;

        // Create brokers
        let diagnostics_broker = DiagnosticsBroker::new();
        let log_broker = LogStreamBroker::new(config.log_allowed_sources);
        let fs_broker = FsBroker::new(config.fs_allowed_patterns);

        let audit_log = AuditLog::new(config.audit_log_path, config.audit_max_size_bytes);

        let agent = Self {
            keypair,
            peer_id,
            capabilities: Arc::new(RwLock::new(HashMap::new())),
            diagnostics_broker,
            log_broker,
            fs_broker,
            policy,
            trust_store,
            audit_log,
            capabilities_dir: config.capabilities_dir,
        };

        // Load any previously installed capabilities
        agent.load_installed_capabilities();

        Ok(agent)
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// Deploy a signed capability to this robot.
    /// Verifies signature, checks trust store, evaluates policy, then installs.
    pub async fn deploy_capability(
        &self,
        manifest_cbor: &[u8],
        component_bytes: &[u8],
        deployer: &PeerId,
    ) -> Result<String, anyhow::Error> {
        // 1. Decode and verify signed manifest
        let signed_manifest = SignedManifest::from_cbor(manifest_cbor)?;
        let manifest = signed_manifest.verify_with_component(component_bytes)?;

        info!(
            name = %manifest.name,
            version = %manifest.version,
            author = %manifest.author_peer_id,
            "Verifying capability for deployment"
        );

        // 2. Check trust store (skip in permissive/dev mode if trust store is empty)
        if !self.trust_store.trusted_peers.is_empty()
            && !self.trust_store.is_trusted(&manifest.author_peer_id)
        {
            return Err(ManifestError::UntrustedSigner {
                peer: manifest.author_peer_id.to_string(),
            }
            .into());
        }

        // 3. Evaluate policy
        self.policy.evaluate(&manifest.declared_capabilities, deployer)?;

        // 4. Store component and manifest
        let cap_dir = self.capabilities_dir.join(&manifest.name);
        std::fs::create_dir_all(&cap_dir)?;

        let wasm_path = cap_dir.join(format!("{}.wasm", manifest.name));
        let manifest_path = cap_dir.join(format!("{}.manifest.cbor", manifest.name));

        std::fs::write(&wasm_path, component_bytes)?;
        std::fs::write(&manifest_path, manifest_cbor)?;

        // 5. Register as installed
        let installed = InstalledCapability {
            name: manifest.name.clone(),
            version: manifest.version,
            author_peer_id: manifest.author_peer_id,
            declared_capabilities: manifest.declared_capabilities,
            component_hash: manifest.component_hash,
            installed_at: chrono::Utc::now(),
            component_path: wasm_path,
            manifest_path,
        };

        self.capabilities
            .write()
            .await
            .insert(manifest.name.clone(), installed);

        info!(name = %manifest.name, "Capability installed");
        Ok(manifest.name)
    }

    /// List installed capabilities.
    pub async fn list_capabilities(&self) -> Vec<InstalledCapability> {
        self.capabilities.read().await.values().cloned().collect()
    }

    /// Invoke an installed capability.
    /// In v0.1, this runs the capability's broker operations directly
    /// (WASM host integration comes in Phase 3).
    pub async fn invoke_capability(
        &self,
        name: &str,
        _args: &[String],
        operator_peer_id: &PeerId,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let caps = self.capabilities.read().await;
        let cap = caps.get(name).ok_or_else(|| {
            CapabilityError::NotFound(name.into())
        })?;

        let started_at = chrono::Utc::now();
        info!(name = %name, operator = %operator_peer_id, "Invoking capability");

        // For v0.1, run broker operations directly based on declared capabilities.
        // Full WASM execution will replace this in Phase 3.
        let mut results = serde_json::Map::new();
        let mut io_stats = Vec::new();

        for group in &cap.declared_capabilities {
            match group {
                CapabilityGroup::DiagnosticsCollect { .. } => {
                    // Collect all diagnostics
                    let sys_req = gang_core::broker::CapabilityRequest {
                        capability_group: "ganglion:diagnostics/collect".into(),
                        operation: gang_core::broker::BrokerOperation::SystemInfo,
                    };
                    if let Ok(resp) = self.diagnostics_broker.handle_request(sys_req).await {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data) {
                            results.insert("system_info".into(), val);
                        }
                        io_stats.push(CapabilityIoStats {
                            capability: group.qualified_name(),
                            bytes_in: resp.bytes_in,
                            bytes_out: resp.bytes_out,
                        });
                    }

                    let proc_req = gang_core::broker::CapabilityRequest {
                        capability_group: "ganglion:diagnostics/collect".into(),
                        operation: gang_core::broker::BrokerOperation::ProcessList,
                    };
                    if let Ok(resp) = self.diagnostics_broker.handle_request(proc_req).await {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data) {
                            results.insert("processes".into(), val);
                        }
                    }

                    let net_req = gang_core::broker::CapabilityRequest {
                        capability_group: "ganglion:diagnostics/collect".into(),
                        operation: gang_core::broker::BrokerOperation::NetworkState,
                    };
                    if let Ok(resp) = self.diagnostics_broker.handle_request(net_req).await {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data) {
                            results.insert("network".into(), val);
                        }
                    }
                }
                CapabilityGroup::LogStream { .. } => {
                    let req = gang_core::broker::CapabilityRequest {
                        capability_group: "ganglion:logs/stream".into(),
                        operation: gang_core::broker::BrokerOperation::LogSourceList,
                    };
                    if let Ok(resp) = self.log_broker.handle_request(req).await {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data) {
                            results.insert("log_sources".into(), val);
                        }
                    }
                }
                CapabilityGroup::RosInterface { .. } => {
                    results.insert(
                        "ros".into(),
                        serde_json::json!({
                            "note": "ROS 2 interface access requires ros2 on the robot",
                        }),
                    );
                }
                _ => {}
            }
        }

        let output = serde_json::to_vec_pretty(&results)?;
        let ended_at = chrono::Utc::now();

        // Write audit record
        let record = AuditRecord {
            operator_peer_id: operator_peer_id.clone(),
            component_name: name.into(),
            component_version: cap.version.clone(),
            component_hash: cap.component_hash.clone(),
            capabilities_used: cap
                .declared_capabilities
                .iter()
                .map(|c| c.qualified_name())
                .collect(),
            started_at,
            ended_at,
            exit_status: ExitStatus::Success,
            io_stats,
        };

        if let Err(e) = self.audit_log.append(&record) {
            warn!("Failed to write audit record: {e}");
        }

        Ok(output)
    }

    /// Load capabilities from the capabilities directory on startup.
    fn load_installed_capabilities(&self) {
        let caps_dir = &self.capabilities_dir;
        if !caps_dir.exists() {
            return;
        }

        let entries = match std::fs::read_dir(caps_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                let manifest_path = entry.path().join(format!("{name}.manifest.cbor"));
                let wasm_path = entry.path().join(format!("{name}.wasm"));

                if manifest_path.exists() && wasm_path.exists() {
                    match std::fs::read(&manifest_path) {
                        Ok(manifest_cbor) => {
                            if let Ok(signed) = SignedManifest::from_cbor(&manifest_cbor) {
                                if let Ok(manifest) = signed.verify_and_decode() {
                                    let installed = InstalledCapability {
                                        name: manifest.name.clone(),
                                        version: manifest.version,
                                        author_peer_id: manifest.author_peer_id,
                                        declared_capabilities: manifest.declared_capabilities,
                                        component_hash: manifest.component_hash,
                                        installed_at: chrono::Utc::now(),
                                        component_path: wasm_path,
                                        manifest_path,
                                    };

                                    // Use try_write to avoid async in sync context
                                    if let Ok(mut caps) = self.capabilities.try_write() {
                                        info!(name = %manifest.name, "Loaded installed capability");
                                        caps.insert(manifest.name, installed);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to read manifest at {}: {e}",
                                manifest_path.display()
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &std::path::Path) -> AgentConfig {
        AgentConfig {
            key_path: dir.join("identity.key"),
            policy_path: None, // permissive
            trust_store_path: dir.join("trusted_peers.json"),
            capabilities_dir: dir.join("capabilities"),
            audit_log_path: dir.join("audit.log"),
            audit_max_size_bytes: 10 * 1024 * 1024,
            fs_allowed_patterns: vec![FsRule {
                pattern: format!("{}/**", dir.display()),
                read: true,
                write: true,
            }],
            log_allowed_sources: vec!["**".into()],
        }
    }

    #[tokio::test]
    async fn agent_creates_and_lists_empty() {
        let dir = TempDir::new().unwrap();
        let agent = RobotAgent::new(test_config(dir.path())).unwrap();
        let caps = agent.list_capabilities().await;
        assert!(caps.is_empty());
    }

    #[tokio::test]
    async fn agent_deploy_and_invoke() {
        let dir = TempDir::new().unwrap();
        let agent = RobotAgent::new(test_config(dir.path())).unwrap();

        // Create a fake capability
        let operator_kp = Keypair::generate();
        let component_bytes = b"fake wasm bytes for testing";
        let component_hash = blake3::hash(component_bytes).to_hex().to_string();

        let manifest = gang_core::manifest::ComponentManifest {
            schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
            name: "test-diag".into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            }],
            author_peer_id: operator_kp.peer_id(),
            component_hash,
            limits: gang_core::manifest::ResourceLimits::default(),
            language: Default::default(),
            description: String::new(),
            tags: vec![],
            min_ganglion_version: None,
        };

        let signed = SignedManifest::sign(&manifest, &operator_kp).unwrap();
        let manifest_cbor = signed.to_cbor().unwrap();

        // Deploy
        let name = agent
            .deploy_capability(&manifest_cbor, component_bytes, &operator_kp.peer_id())
            .await
            .unwrap();
        assert_eq!(name, "test-diag");

        // List
        let caps = agent.list_capabilities().await;
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].name, "test-diag");

        // Invoke
        let output = agent
            .invoke_capability("test-diag", &[], &operator_kp.peer_id())
            .await
            .unwrap();

        // Should have system_info in the output
        let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert!(result.get("system_info").is_some());
    }

    #[tokio::test]
    async fn agent_rejects_untrusted_signer() {
        let dir = TempDir::new().unwrap();

        // Create agent with a non-empty trust store that doesn't include our signer
        let trusted_kp = Keypair::generate();
        let mut trust_store = TrustStore::default();
        trust_store.add(gang_core::manifest::TrustedPeer {
            peer_id: trusted_kp.peer_id(),
            name: "trusted-operator".into(),
            public_key: trusted_kp.public_key().to_bytes().to_vec(),
        });
        let trust_path = dir.path().join("trusted_peers.json");
        trust_store.save(&trust_path).unwrap();

        let mut config = test_config(dir.path());
        config.trust_store_path = trust_path;
        let agent = RobotAgent::new(config).unwrap();

        // Try to deploy with an untrusted key
        let untrusted_kp = Keypair::generate();
        let component = b"fake wasm";
        let manifest = gang_core::manifest::ComponentManifest {
            schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
            name: "evil-cap".into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![],
            author_peer_id: untrusted_kp.peer_id(),
            component_hash: blake3::hash(component).to_hex().to_string(),
            limits: Default::default(),
            language: Default::default(),
            description: String::new(),
            tags: vec![],
            min_ganglion_version: None,
        };
        let signed = SignedManifest::sign(&manifest, &untrusted_kp).unwrap();
        let manifest_cbor = signed.to_cbor().unwrap();

        let result = agent
            .deploy_capability(&manifest_cbor, component, &untrusted_kp.peer_id())
            .await;
        assert!(result.is_err());
    }
}
