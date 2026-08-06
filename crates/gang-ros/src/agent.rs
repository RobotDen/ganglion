use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::{info, warn};

use gang_core::audit::{AuditLog, AuditRecord, CapabilityIoStats, ExitStatus};
use gang_core::broker::CapabilityBroker;
use gang_core::capability::{CapabilityGroup, InstalledCapability};
use gang_core::error::{CapabilityError, ManifestError};
use gang_core::events::{AgentEvent, AuditProjection, EventSubscribeRequest, PolicyOutcome};
use gang_core::identity::{Keypair, PeerId};
use gang_core::manifest::{SignedManifest, TrustStore};
use gang_core::policy::Policy;

use crate::diagnostics::DiagnosticsBroker;
use crate::events::{EventBus, SubscribeError};
use crate::filesystem::{FsBroker, FsRule};
use crate::logs::LogStreamBroker;

/// How often the agent emits a [`AgentEvent::Heartbeat`] to the event bus.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// The robot agent — the central process running on a deployed robot.
/// Manages installed capabilities, brokers, policy enforcement, and audit logging.
#[allow(dead_code)] // keypair retained for future outbound signing
pub struct RobotAgent {
    /// This robot's identity.
    keypair: Keypair,
    peer_id: PeerId,

    /// Installed capabilities indexed by name.
    capabilities: Arc<RwLock<HashMap<String, InstalledCapability>>>,

    /// Layer 3 brokers. Held as `Arc` so the exact same instances (and their
    /// configured rules) are shared with the WASM component runtime (SEC-06).
    diagnostics_broker: Arc<DiagnosticsBroker>,
    log_broker: Arc<LogStreamBroker>,
    fs_broker: Arc<FsBroker>,

    /// WASM component runtime, constructed once with the shared engine and the
    /// agent's real broker set (CODE-06 / SEC-06).
    runtime: Arc<gang_wasm_host::runtime::ComponentRuntime>,

    /// Policy engine.
    policy: Policy,

    /// Trust store for verifying capability signatures.
    trust_store: TrustStore,

    /// Audit log.
    audit_log: AuditLog,

    /// Directory for storing installed capabilities.
    capabilities_dir: PathBuf,

    /// Replay-protection guard for inbound control requests (SEC-14). Rejects
    /// requests whose nonce has been seen or whose timestamp is outside the
    /// freshness window.
    replay_guard: Arc<Mutex<gang_core::message::ReplayGuard>>,

    /// Bounded in-process event bus feeding the `/ganglion/events/1.0`
    /// subscription (presence, policy decisions, audit appends, heartbeats).
    event_bus: Arc<EventBus>,

    /// When the agent was constructed, for reporting uptime in presence
    /// snapshots and heartbeats.
    started_at: Instant,
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

        // SEC-01: policy load must FAIL CLOSED. A configured-but-unreadable or
        // malformed policy file is a fatal error — we must never silently fall
        // back to a permissive policy that would allow everything. Permissive
        // is kept ONLY for the explicit "no policy configured" dev path, and
        // that path is made loud.
        let policy = match config.policy_path {
            Some(ref path) => Policy::load(path).map_err(|e| {
                anyhow::anyhow!(
                    "failed to load policy from {} (refusing to start with a permissive \
                     fallback): {e}",
                    path.display()
                )
            })?,
            None => {
                warn!(
                    "No policy file configured — using PERMISSIVE policy. This allows ALL \
                     capability requests and is intended for development only."
                );
                Policy::permissive()
            }
        };

        // SEC-01: trust-store load must FAIL CLOSED. `TrustStore::load` returns
        // an empty store when the file is simply absent (the dev path); any
        // genuine read/parse error is fatal — we must not silently start with
        // an empty trust store that would disable signature trust checks.
        let trust_store = TrustStore::load(&config.trust_store_path).map_err(|e| {
            anyhow::anyhow!(
                "failed to load trust store from {} (refusing to start with an empty \
                 fallback): {e}",
                config.trust_store_path.display()
            )
        })?;
        if trust_store.trusted_peers.is_empty() {
            warn!(
                "Trust store is empty — capability signature trust checks are DISABLED. \
                 This is intended for development only."
            );
        }

        // Create capabilities directory
        std::fs::create_dir_all(&config.capabilities_dir)?;

        // SEC-06: create the real broker instances once, wrapped in Arc, so the
        // exact same instances (with their configured rules) are shared with
        // the WASM runtime rather than synthesizing permissive ones per invoke.
        let diagnostics_broker = Arc::new(DiagnosticsBroker::new());
        let log_broker = Arc::new(LogStreamBroker::new(config.log_allowed_sources));
        let fs_broker = Arc::new(FsBroker::new(config.fs_allowed_patterns));

        // CODE-06: build the wasmtime engine + component runtime ONCE. The
        // engine owns a single epoch-ticker thread and a compilation cache; the
        // runtime caches compiled components. Both live for the agent's
        // lifetime instead of being rebuilt on every invoke_capability call.
        let engine = gang_wasm_host::GanglionEngine::new()?;
        let mut brokers: HashMap<String, Arc<dyn CapabilityBroker>> = HashMap::new();
        brokers.insert(
            "ganglion:diagnostics/collect".into(),
            diagnostics_broker.clone(),
        );
        brokers.insert("ganglion:logs/stream".into(), log_broker.clone());
        brokers.insert("ganglion:fs/bounded".into(), fs_broker.clone());
        let runtime = Arc::new(gang_wasm_host::runtime::ComponentRuntime::new(
            engine, brokers,
        )?);

        let audit_log = AuditLog::new(config.audit_log_path, config.audit_max_size_bytes);

        let mut agent = Self {
            keypair,
            peer_id,
            capabilities: Arc::new(RwLock::new(HashMap::new())),
            diagnostics_broker,
            log_broker,
            fs_broker,
            runtime,
            policy,
            trust_store,
            audit_log,
            capabilities_dir: config.capabilities_dir,
            // SEC-14: 5-minute freshness window for inbound requests.
            replay_guard: Arc::new(Mutex::new(gang_core::message::ReplayGuard::new(
                Duration::from_secs(300),
            ))),
            event_bus: Arc::new(EventBus::default()),
            started_at: Instant::now(),
        };

        // Load any previously installed capabilities (CODE-21: propagate errors).
        agent.load_installed_capabilities()?;

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

        // 2. Check trust store (skip in permissive/dev mode if trust store is empty).
        //    Both the manifest SIGNER and the connecting DEPLOYER must be trusted
        //    (SEC-03): the signer proves the bundle is authentic, the deployer
        //    proves the peer pushing it to this robot is authorized. Because the
        //    deployer id is now derived from the same raw Ed25519 key on the wire
        //    as in the trust store, this check is actually enforceable.
        if !self.trust_store.trusted_peers.is_empty() {
            if !self.trust_store.is_trusted(&manifest.author_peer_id) {
                return Err(ManifestError::UntrustedSigner {
                    peer: manifest.author_peer_id.to_string(),
                }
                .into());
            }
            if !self.trust_store.is_trusted(deployer) {
                return Err(ManifestError::UntrustedSigner {
                    peer: format!("deployer {deployer}"),
                }
                .into());
            }
        }

        // 3. Evaluate policy against the authenticated deployer identity.
        //    Emit a PolicyDecision on BOTH paths so the event feed records every
        //    evaluation, not just the failures.
        let group_summary = manifest
            .declared_capabilities
            .iter()
            .map(|g| g.name())
            .collect::<Vec<_>>()
            .join(",");
        match self
            .policy
            .evaluate(&manifest.declared_capabilities, deployer)
        {
            Ok(()) => {
                let deployer = deployer.clone();
                let group_summary = group_summary.clone();
                self.event_bus
                    .publish(move |seq| AgentEvent::PolicyDecision {
                        seq,
                        ts: chrono::Utc::now(),
                        operator_peer: deployer,
                        capability_group: group_summary,
                        decision: PolicyOutcome::Allow,
                        reason: "capabilities permitted by policy".into(),
                    });
            }
            Err(e) => {
                let reason = e.to_string();
                let deployer_ev = deployer.clone();
                self.event_bus
                    .publish(move |seq| AgentEvent::PolicyDecision {
                        seq,
                        ts: chrono::Utc::now(),
                        operator_peer: deployer_ev,
                        capability_group: group_summary,
                        decision: PolicyOutcome::Deny,
                        reason,
                    });
                return Err(e.into());
            }
        }

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
    ///
    /// Prefer WASM execution when component bytes are available. Fall back to
    /// direct broker invocation for non-WASM capabilities (e.g., when the
    /// component file is missing or contains placeholder bytes from testing).
    pub async fn invoke_capability(
        &self,
        name: &str,
        args: &[String],
        operator_peer_id: &PeerId,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let caps = self.capabilities.read().await;
        let cap = caps
            .get(name)
            .ok_or_else(|| CapabilityError::NotFound(name.into()))?;

        let started_at = chrono::Utc::now();
        info!(name = %name, operator = %operator_peer_id, "Invoking capability");

        // Prefer WASM execution when component bytes are available.
        // Read the component file and, if it looks like a valid WASM binary
        // (starts with the \0asm magic bytes), run it through the
        // ComponentRuntime. This is the intended three-layer execution path:
        //   WASM component -> host functions -> Layer 3 brokers.
        let wasm_bytes = std::fs::read(&cap.component_path).ok();
        let is_valid_wasm = wasm_bytes
            .as_ref()
            .map(|b| b.len() > 8 && b.starts_with(b"\0asm"))
            .unwrap_or(false);

        if is_valid_wasm {
            let wasm_bytes = wasm_bytes.unwrap();

            // SEC-07: re-hash the on-disk component bytes immediately before
            // execution and compare against the manifest's component_hash. A
            // mismatch means the file was tampered with after install — reject
            // and audit the failure, never execute.
            let actual_hash = blake3::hash(&wasm_bytes).to_hex().to_string();
            if actual_hash != cap.component_hash {
                warn!(
                    name = %name,
                    expected = %cap.component_hash,
                    actual = %actual_hash,
                    "Component hash mismatch — refusing to execute tampered component"
                );
                self.record_audit(
                    operator_peer_id,
                    name,
                    cap,
                    started_at,
                    ExitStatus::Failed {
                        message: format!(
                            "component hash mismatch: expected {}, found {actual_hash}",
                            cap.component_hash
                        ),
                    },
                    vec![],
                );
                return Err(ManifestError::HashMismatch {
                    expected: cap.component_hash.clone(),
                    actual: actual_hash,
                }
                .into());
            }

            info!(name = %name, "Executing via WASM component runtime");

            // SEC-05: pass the capability's stored manifest ResourceLimits
            // (fuel / epoch / memory) into the invocation. The runtime clamps
            // each value to the host maxima.
            let limits = self.manifest_limits(cap);

            // SEC-06: the shared runtime already holds the agent's real,
            // configured broker instances — no permissive brokers are
            // synthesized here.
            let result = self
                .runtime
                .invoke(
                    &wasm_bytes,
                    &cap.component_hash,
                    cap.declared_capabilities.clone(),
                    &limits,
                    args.to_vec(),
                )
                .await;

            // SEC-13: a WASM trap / fuel exhaustion / deadline / policy denial
            // is a terminal, audited error returned to the caller. We do NOT
            // fall back to ambient broker invocation.
            match result {
                Ok(comp_result) => {
                    self.record_audit(
                        operator_peer_id,
                        name,
                        cap,
                        started_at,
                        ExitStatus::Success,
                        vec![],
                    );
                    return Ok(comp_result.data);
                }
                Err(e) => {
                    let exit_status = wasm_exit_status(&e);
                    warn!(
                        name = %name,
                        error = %e,
                        "WASM execution failed — terminal error (no broker fallback)"
                    );
                    self.record_audit(operator_peer_id, name, cap, started_at, exit_status, vec![]);
                    return Err(anyhow::anyhow!("WASM execution of '{name}' failed: {e}"));
                }
            }
        }

        // Direct broker invocation for non-WASM capabilities. This is reached
        // ONLY when there is no valid WASM binary on disk (e.g., test fixtures
        // or placeholder bytes) — it is NOT a fallback from a failed WASM run.
        let mut results = serde_json::Map::new();
        let mut io_stats = Vec::new();

        for group in &cap.declared_capabilities {
            match group {
                CapabilityGroup::DiagnosticsCollect { .. } => {
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
                    if let Ok(resp) = self.diagnostics_broker.handle_request(proc_req).await
                        && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data)
                    {
                        results.insert("processes".into(), val);
                    }

                    let net_req = gang_core::broker::CapabilityRequest {
                        capability_group: "ganglion:diagnostics/collect".into(),
                        operation: gang_core::broker::BrokerOperation::NetworkState,
                    };
                    if let Ok(resp) = self.diagnostics_broker.handle_request(net_req).await
                        && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data)
                    {
                        results.insert("network".into(), val);
                    }
                }
                CapabilityGroup::LogStream { .. } => {
                    let req = gang_core::broker::CapabilityRequest {
                        capability_group: "ganglion:logs/stream".into(),
                        operation: gang_core::broker::BrokerOperation::LogSourceList,
                    };
                    if let Ok(resp) = self.log_broker.handle_request(req).await
                        && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.data)
                    {
                        results.insert("log_sources".into(), val);
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

        self.record_audit(
            operator_peer_id,
            name,
            cap,
            started_at,
            ExitStatus::Success,
            io_stats,
        );

        Ok(output)
    }

    /// Load the manifest-declared resource limits for an installed capability.
    ///
    /// The limits are stored inside the on-disk signed manifest; we decode it
    /// on demand (SEC-05). If the manifest cannot be read/decoded we fall back
    /// to defaults — the runtime clamps every value to the host maxima anyway.
    fn manifest_limits(&self, cap: &InstalledCapability) -> gang_core::manifest::ResourceLimits {
        match std::fs::read(&cap.manifest_path)
            .ok()
            .and_then(|bytes| SignedManifest::from_cbor(&bytes).ok())
            .and_then(|signed| signed.verify_and_decode().ok())
        {
            Some(manifest) => manifest.limits,
            None => {
                warn!(
                    name = %cap.name,
                    "Could not read manifest limits; using host defaults"
                );
                gang_core::manifest::ResourceLimits::default()
            }
        }
    }

    /// Append an audit record for a capability invocation. Failures to persist
    /// the record are logged but do not mask the invocation result.
    fn record_audit(
        &self,
        operator_peer_id: &PeerId,
        name: &str,
        cap: &InstalledCapability,
        started_at: chrono::DateTime<chrono::Utc>,
        exit_status: ExitStatus,
        io_stats: Vec<CapabilityIoStats>,
    ) {
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
            ended_at: chrono::Utc::now(),
            exit_status,
            io_stats,
        };
        if let Err(e) = self.audit_log.append(&record) {
            warn!("Failed to write audit record: {e}");
        }

        // A policy denial surfaced by the sandbox at invoke time (e.g. a WASM
        // guest reaching for an undeclared capability) is a policy decision as
        // much as a deploy-time one — emit it on the feed's deny path too.
        if let ExitStatus::PolicyDenied { reason } = &record.exit_status {
            let operator = record.operator_peer_id.clone();
            let group = record.capabilities_used.join(",");
            let reason = reason.clone();
            self.event_bus
                .publish(move |seq| AgentEvent::PolicyDecision {
                    seq,
                    ts: chrono::Utc::now(),
                    operator_peer: operator,
                    capability_group: group,
                    decision: PolicyOutcome::Deny,
                    reason,
                });
        }

        // Every appended record is announced on the feed (secret-free
        // projection) so `gang logs` and live viewers see it.
        let projection = AuditProjection::from(&record);
        self.event_bus
            .publish(move |seq| AgentEvent::AuditAppended {
                seq,
                record: projection,
            });
    }

    /// Access the agent's in-process event bus (for live in-process consumers
    /// and tests).
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }

    /// Agent uptime in whole seconds.
    fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Build the ordered event batch for a subscription request, enforcing the
    /// subscriber trust rule first.
    ///
    /// Authentication mirrors deploy (SEC-03): when a trust store is
    /// configured, only trusted operators may subscribe; an empty trust store
    /// is the loud dev-permissive path. On a fresh subscription (`since_seq`
    /// is `None`) the batch is headed by a [`AgentEvent::PresenceSnapshot`];
    /// otherwise it carries only events newer than the cursor. A dropped-events
    /// gap (the subscriber fell behind the retained window) is surfaced as a
    /// leading [`AgentEvent::Gap`].
    pub async fn build_event_subscription(
        &self,
        subscriber: &PeerId,
        req: &EventSubscribeRequest,
    ) -> Result<Vec<AgentEvent>, SubscribeError> {
        if !self.trust_store.trusted_peers.is_empty() && !self.trust_store.is_trusted(subscriber) {
            warn!(peer = %subscriber, "Rejecting event subscription from untrusted peer");
            return Err(SubscribeError::Unauthorized {
                peer: subscriber.to_string(),
            });
        }

        let mut out = Vec::new();
        if req.since_seq.is_none() {
            out.push(AgentEvent::PresenceSnapshot {
                seq: self.event_bus.tip(),
                ganglion_version: env!("CARGO_PKG_VERSION").to_string(),
                uptime_secs: self.uptime_secs(),
                archetype: None,
                installed_capabilities: self
                    .list_capabilities()
                    .await
                    .into_iter()
                    .map(|c| c.name)
                    .collect(),
            });
        }

        let max = req
            .max_events
            .map(|m| m as usize)
            .unwrap_or(crate::events::RING_CAPACITY)
            .min(crate::events::RING_CAPACITY);
        let batch = self.event_bus.recent_since(req.since_seq, max);
        if batch.dropped > 0 {
            out.push(AgentEvent::Gap {
                dropped: batch.dropped,
            });
        }
        out.extend(batch.events);
        Ok(out)
    }

    /// Start serving incoming control messages and the event push feed.
    ///
    /// Registers a request-response handler on `/ganglion/control/1.0` (deploy,
    /// invoke, list, …) and accepts genuine push substreams on
    /// `/ganglion/events/1.0` (ADR-024): each subscriber that opens the event
    /// protocol is authenticated and then has framed [`AgentEvent`]s pushed to
    /// it live from the bounded [`EventBus`] until the stream closes.
    ///
    /// Takes the concrete [`gang_libp2p::Libp2pTransportAdapter`] because the
    /// push feed uses its raw-substream accept API, which is intentionally not
    /// part of the transport-agnostic `TransportAdapter` trait (that trait stays
    /// dependency-free in `gang-core`).
    pub async fn serve(
        self: &Arc<Self>,
        transport: &gang_libp2p::Libp2pTransportAdapter,
    ) -> anyhow::Result<()> {
        use gang_core::message::{
            CapabilityInfo, ControlMessage, InvokeStatus, decode_message, encode_message,
        };
        use gang_core::protocol::ProtocolId;
        use gang_core::transport::{StreamHandler, TransportAdapter};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let agent = Arc::clone(self);

        let handler: StreamHandler = Box::new(move |mut stream| {
            let agent = Arc::clone(&agent);
            Box::pin(async move {
                // Read all request bytes from the stream
                let mut request_bytes = Vec::new();
                if let Err(e) = stream.inner.read_to_end(&mut request_bytes).await {
                    warn!("Failed to read request from {}: {e}", stream.remote_peer);
                    return;
                }

                if request_bytes.is_empty() {
                    return;
                }

                // Decode the control message
                let msg: ControlMessage = match decode_message(&request_bytes) {
                    Ok((msg, _)) => msg,
                    Err(e) => {
                        warn!(
                            "Failed to decode control message from {}: {e}",
                            stream.remote_peer
                        );
                        return;
                    }
                };

                let remote_peer = stream.remote_peer.clone();
                info!(peer = %remote_peer, "Received control message");

                // SEC-14: reject stale or replayed requests before dispatch.
                if let Some((nonce, timestamp_ms)) = msg.request_meta() {
                    let verdict = agent
                        .replay_guard
                        .lock()
                        .expect("replay guard poisoned")
                        .observe(nonce, timestamp_ms);
                    if let Err(e) = verdict {
                        warn!(peer = %remote_peer, "Rejecting request (replay/stale): {e}");
                        let response = ControlMessage::Error {
                            request_id: None,
                            code: "replay_rejected".into(),
                            message: e.to_string(),
                        };
                        if let Ok(bytes) = encode_message(&response) {
                            let _ = stream.inner.write_all(&bytes).await;
                        }
                        return;
                    }
                }

                // Dispatch and build response
                let response: ControlMessage = match msg {
                    ControlMessage::DeployCapability {
                        manifest_cbor,
                        component_bytes,
                        ..
                    } => {
                        match agent
                            .deploy_capability(&manifest_cbor, &component_bytes, &remote_peer)
                            .await
                        {
                            Ok(name) => ControlMessage::InvokeResult {
                                request_id: String::new(),
                                status: InvokeStatus::Success,
                                output: name.into_bytes(),
                            },
                            Err(e) => ControlMessage::Error {
                                request_id: None,
                                code: "deploy_failed".into(),
                                message: e.to_string(),
                            },
                        }
                    }
                    ControlMessage::InvokeCapability {
                        name,
                        args,
                        request_id,
                        ..
                    } => match agent.invoke_capability(&name, &args, &remote_peer).await {
                        Ok(output) => ControlMessage::InvokeResult {
                            request_id,
                            status: InvokeStatus::Success,
                            output,
                        },
                        Err(e) => ControlMessage::Error {
                            request_id: Some(request_id),
                            code: "invoke_failed".into(),
                            message: e.to_string(),
                        },
                    },
                    ControlMessage::ListCapabilities => {
                        let caps = agent.list_capabilities().await;
                        ControlMessage::CapabilityList {
                            capabilities: caps
                                .into_iter()
                                .map(|c| CapabilityInfo {
                                    name: c.name,
                                    version: c.version,
                                    author: c.author_peer_id,
                                    declared_capabilities: c
                                        .declared_capabilities
                                        .iter()
                                        .map(|g| g.qualified_name())
                                        .collect(),
                                })
                                .collect(),
                        }
                    }
                    ControlMessage::Presence { .. } => {
                        info!(peer = %remote_peer, "Received presence announcement");
                        return; // No response for presence
                    }
                    _ => ControlMessage::Error {
                        request_id: None,
                        code: "unexpected_message".into(),
                        message: "unexpected message type for robot agent".into(),
                    },
                };

                // Encode and write response
                match encode_message(&response) {
                    Ok(response_bytes) => {
                        if let Err(e) = stream.inner.write_all(&response_bytes).await {
                            warn!("Failed to write response to {}: {e}", remote_peer);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to encode response: {e}");
                    }
                }
            })
        });

        transport
            .listen(ProtocolId::control(), handler)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to register control handler: {e}"))?;

        // --- /ganglion/events/1.0: genuine push substreams (ADR-024) ---
        // Accept persistent inbound event substreams and push framed events to
        // each authenticated subscriber live, rather than serving a buffered
        // request-response batch.
        let mut incoming = transport
            .accept_event_streams()
            .map_err(|e| anyhow::anyhow!("Failed to accept event streams: {e}"))?;
        let events_agent = Arc::clone(self);
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some((subscriber, stream)) = incoming.next().await {
                let agent = Arc::clone(&events_agent);
                // One task per subscriber so a slow/stalled operator only
                // delays its own feed, never the agent or its peers.
                tokio::spawn(async move {
                    push_event_feed(agent, subscriber, stream).await;
                });
            }
        });

        // Heartbeat ticker: emit a liveness beat on the bus every N seconds so
        // a live viewer can tell "quiet" from "gone". Lives for the process.
        let hb_agent = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let uptime = hb_agent.uptime_secs();
                hb_agent
                    .event_bus
                    .publish(move |seq| AgentEvent::Heartbeat {
                        seq,
                        ts: chrono::Utc::now(),
                        uptime_secs: uptime,
                    });
            }
        });

        // Bridge transport connection events onto the bus as ConnectionChanged.
        let conn_bus = Arc::clone(&self.event_bus);
        let mut transport_events = transport.events();
        tokio::spawn(async move {
            use futures::StreamExt;
            use gang_core::events::ConnectionState;
            use gang_core::transport::TransportEvent;
            while let Some(ev) = transport_events.next().await {
                match ev {
                    TransportEvent::PeerConnected { peer_id, via_relay } => {
                        conn_bus.publish(move |seq| AgentEvent::ConnectionChanged {
                            seq,
                            ts: chrono::Utc::now(),
                            peer: peer_id,
                            transport: if via_relay {
                                "relay".into()
                            } else {
                                "direct".into()
                            },
                            via_relay,
                            state: ConnectionState::Up,
                        });
                    }
                    TransportEvent::PeerDisconnected { peer_id } => {
                        conn_bus.publish(move |seq| AgentEvent::ConnectionChanged {
                            seq,
                            ts: chrono::Utc::now(),
                            peer: peer_id,
                            transport: "".into(),
                            via_relay: false,
                            state: ConnectionState::Down,
                        });
                    }
                    TransportEvent::DirectUpgrade { peer_id } => {
                        conn_bus.publish(move |seq| AgentEvent::ConnectionChanged {
                            seq,
                            ts: chrono::Utc::now(),
                            peer: peer_id,
                            transport: "direct".into(),
                            via_relay: false,
                            state: ConnectionState::Up,
                        });
                    }
                    _ => {}
                }
            }
        });

        info!(
            peer_id = %self.peer_id,
            "Robot agent serving on /ganglion/control/1.0 and /ganglion/events/1.0"
        );
        Ok(())
    }

    /// Load capabilities from the capabilities directory on startup.
    ///
    /// CODE-21: takes `&mut self` and inserts verified capabilities directly
    /// into the (not-yet-shared) map, so a lock contention can never silently
    /// drop a verified capability. Genuine infrastructure failures (directory
    /// read error, unexpectedly-shared map) return an error.
    ///
    /// SEC-07: each component's on-disk bytes are re-hashed and checked against
    /// the manifest's `component_hash` via `verify_with_component` (not just the
    /// signature) before the capability is registered.
    fn load_installed_capabilities(&mut self) -> anyhow::Result<()> {
        let caps_dir = self.capabilities_dir.clone();
        if !caps_dir.exists() {
            return Ok(());
        }

        let entries = std::fs::read_dir(&caps_dir).map_err(|e| {
            anyhow::anyhow!(
                "failed to read capabilities directory {}: {e}",
                caps_dir.display()
            )
        })?;

        let trust_store = self.trust_store.clone();
        let mut loaded: Vec<(String, InstalledCapability)> = Vec::new();

        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let manifest_path = entry.path().join(format!("{name}.manifest.cbor"));
            let wasm_path = entry.path().join(format!("{name}.wasm"));

            if !manifest_path.exists() || !wasm_path.exists() {
                continue;
            }

            let manifest_cbor = match std::fs::read(&manifest_path) {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        "Failed to read manifest at {}: {e}",
                        manifest_path.display()
                    );
                    continue;
                }
            };
            let signed = match SignedManifest::from_cbor(&manifest_cbor) {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                        "Failed to deserialize manifest at {}: {e}",
                        manifest_path.display()
                    );
                    continue;
                }
            };

            // SEC-07: re-hash the on-disk component and verify it matches the
            // signed manifest — reject signature-valid-but-tampered components.
            let component_bytes = match std::fs::read(&wasm_path) {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to read component at {}: {e}", wasm_path.display());
                    continue;
                }
            };
            let manifest = match signed.verify_with_component(&component_bytes) {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        "Verification failed for {} (signature or component hash): {e}",
                        manifest_path.display()
                    );
                    continue;
                }
            };

            // Verify against trust store (skip check if empty — dev mode).
            if !trust_store.trusted_peers.is_empty()
                && !trust_store.is_trusted(&manifest.author_peer_id)
            {
                warn!(
                    name = %manifest.name,
                    author = %manifest.author_peer_id,
                    "Skipping capability: author not in trust store"
                );
                continue;
            }

            let cap_name = manifest.name.clone();
            let installed = InstalledCapability {
                name: manifest.name,
                version: manifest.version,
                author_peer_id: manifest.author_peer_id,
                declared_capabilities: manifest.declared_capabilities,
                component_hash: manifest.component_hash,
                installed_at: chrono::Utc::now(),
                component_path: wasm_path,
                manifest_path,
            };
            info!(name = %cap_name, "Loaded installed capability");
            loaded.push((cap_name, installed));
        }

        // The capabilities map is not yet shared (the agent has just been
        // constructed and not cloned), so `get_mut` gives us exclusive access
        // without locking. This is the CODE-21 fix: no `try_write` that could
        // silently fail under contention.
        let map = Arc::get_mut(&mut self.capabilities)
            .ok_or_else(|| {
                anyhow::anyhow!("capabilities map was unexpectedly shared during startup load")
            })?
            .get_mut();
        for (name, cap) in loaded {
            map.insert(name, cap);
        }

        Ok(())
    }
}

/// Serve one operator's `/ganglion/events/1.0` push substream (ADR-024):
/// authenticate, send the presence snapshot + retained catch-up, then push
/// framed [`AgentEvent`]s live from the bounded [`EventBus`] until the stream
/// closes.
///
/// The subscriber identity is the wire-authenticated gang id libp2p proved on
/// the connection (SEC-03), never a self-report. Authorization reuses
/// [`RobotAgent::build_event_subscription`] — the SAME trust rule as deploy
/// (trusted-only when a trust store is configured; loud dev-permissive when
/// empty). An unauthorized subscriber is streamed NOTHING: the stream is
/// dropped (closed) without a snapshot or any events.
///
/// Slow/lagging live consumers degrade to an [`AgentEvent::Gap`] marker via the
/// bounded broadcast bus, never unbounded robot memory.
async fn push_event_feed<S>(agent: Arc<RobotAgent>, subscriber: PeerId, mut stream: S)
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin + Send,
{
    use gang_core::message::{decode_message, encode_message};
    use gang_libp2p::framed::{read_frame, write_frame};

    // Subscribe to the live feed FIRST, so events emitted during the snapshot
    // build are captured on the broadcast channel (deduplicated below) rather
    // than lost in the window between snapshot and tail.
    let mut live = agent.event_bus().subscribe();

    // Read the single subscription request frame. A clean EOF (operator opened
    // and closed its write half) is treated as a fresh default subscription.
    let req = match read_frame(&mut stream).await {
        Ok(Some(frame)) => match decode_message::<EventSubscribeRequest>(&frame) {
            Ok((req, _)) => req,
            Err(e) => {
                warn!(peer = %subscriber, "Failed to decode subscribe request: {e}");
                return;
            }
        },
        Ok(None) => EventSubscribeRequest::default(),
        Err(e) => {
            warn!(peer = %subscriber, "Failed to read subscribe request: {e}");
            return;
        }
    };

    // Authenticate + build the snapshot/catch-up batch. An unauthorized peer is
    // refused here: we return (dropping the stream) so it is streamed nothing.
    let batch = match agent.build_event_subscription(&subscriber, &req).await {
        Ok(events) => events,
        Err(e) => {
            warn!(peer = %subscriber, "Event subscription refused: {e}");
            return;
        }
    };

    // Push the catch-up batch, tracking the highest REAL event sequence sent
    // (the presence snapshot carries the tip as an indicator, not a delivered
    // event, so it does not advance the dedup boundary).
    let mut last_seq: Option<u64> = None;
    for ev in &batch {
        match encode_message(ev) {
            Ok(frame) => {
                if write_frame(&mut stream, &frame).await.is_err() {
                    return;
                }
            }
            Err(e) => {
                warn!(peer = %subscriber, "Failed to encode event: {e}");
                return;
            }
        }
        if !matches!(ev, AgentEvent::PresenceSnapshot { .. })
            && let Some(s) = ev.seq()
        {
            last_seq = Some(last_seq.map_or(s, |l| l.max(s)));
        }
    }

    // Live tail: push each freshly emitted event as it arrives. Skip any event
    // already delivered in the catch-up batch (overlap dedup). A lagging
    // consumer surfaces as a Gap (no seq) and is always forwarded.
    while let Some(ev) = live.recv().await {
        if let (Some(s), Some(last)) = (ev.seq(), last_seq)
            && s <= last
        {
            continue;
        }
        match encode_message(&ev) {
            Ok(frame) => {
                if write_frame(&mut stream, &frame).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                warn!(peer = %subscriber, "Failed to encode live event: {e}");
                break;
            }
        }
        if let Some(s) = ev.seq() {
            last_seq = Some(last_seq.map_or(s, |l| l.max(s)));
        }
    }
}

/// Map a WASM [`InvocationError`](gang_wasm_host::InvocationError) to an honest
/// audit [`ExitStatus`] (SEC-13 / SEC-08). Fuel exhaustion and deadlines are
/// recorded distinctly, undeclared-capability access is a policy denial, and
/// everything else is a trap/failure.
fn wasm_exit_status(err: &gang_wasm_host::InvocationError) -> ExitStatus {
    use gang_wasm_host::InvocationError as IE;
    match err {
        IE::DeadlineExceeded { .. } => ExitStatus::Timeout,
        IE::Trapped(msg) => ExitStatus::Trapped {
            message: msg.clone(),
        },
        IE::UndeclaredCapability(msg) => ExitStatus::PolicyDenied {
            reason: msg.clone(),
        },
        IE::FuelExhausted { consumed } => ExitStatus::Failed {
            message: format!("fuel exhausted after {consumed} units"),
        },
        other => ExitStatus::Failed {
            message: other.to_string(),
        },
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

    // --- Event subscription: trust + emission ---

    /// Configure an agent with a non-empty trust store containing `trusted`.
    fn agent_with_trusted(dir: &std::path::Path, trusted: &Keypair) -> RobotAgent {
        let mut trust = TrustStore::default();
        trust.add(gang_core::manifest::TrustedPeer {
            peer_id: trusted.peer_id(),
            name: "trusted-op".into(),
            public_key: trusted.public_key().to_bytes().to_vec(),
        });
        let trust_path = dir.join("trusted_peers.json");
        trust.save(&trust_path).unwrap();
        let mut config = test_config(dir);
        config.trust_store_path = trust_path;
        RobotAgent::new(config).unwrap()
    }

    #[tokio::test]
    async fn unauthorized_peer_cannot_subscribe() {
        let dir = TempDir::new().unwrap();
        let trusted = Keypair::generate();
        let agent = agent_with_trusted(dir.path(), &trusted);

        // A stranger (not in the trust store) is refused.
        let stranger = Keypair::generate().peer_id();
        let err = agent
            .build_event_subscription(&stranger, &EventSubscribeRequest::fresh())
            .await
            .expect_err("stranger must be refused");
        assert!(matches!(err, SubscribeError::Unauthorized { .. }));

        // The trusted operator gets a batch headed by a presence snapshot.
        let ok = agent
            .build_event_subscription(&trusted.peer_id(), &EventSubscribeRequest::fresh())
            .await
            .expect("trusted operator may subscribe");
        assert!(matches!(
            ok.first(),
            Some(AgentEvent::PresenceSnapshot { .. })
        ));
    }

    #[tokio::test]
    async fn empty_trust_store_is_dev_permissive_for_subscribe() {
        let dir = TempDir::new().unwrap();
        // test_config uses an absent (empty) trust store → dev-permissive.
        let agent = RobotAgent::new(test_config(dir.path())).unwrap();
        let anyone = Keypair::generate().peer_id();
        let batch = agent
            .build_event_subscription(&anyone, &EventSubscribeRequest::fresh())
            .await
            .expect("dev-permissive path allows any subscriber");
        assert!(matches!(
            batch.first(),
            Some(AgentEvent::PresenceSnapshot { .. })
        ));
    }

    #[tokio::test]
    async fn deploy_deny_emits_policy_decision_on_feed() {
        let dir = TempDir::new().unwrap();
        // Restrictive policy: only diagnostics is permitted; process/spawn is
        // denied. The deployer is authorized to deploy.
        let policy_toml = r#"
[[capability_rules]]
group = "ganglion:diagnostics/collect"
allowed_patterns = ["**"]

[[peer_rules]]
peer_id = "*"
can_deploy = true
"#;
        let policy_path = dir.path().join("policy.toml");
        std::fs::write(&policy_path, policy_toml).unwrap();
        let mut config = test_config(dir.path());
        config.policy_path = Some(policy_path);
        let agent = RobotAgent::new(config).unwrap();

        // Deploy a component declaring an UNDECLARED-by-policy capability group.
        let kp = Keypair::generate();
        let component = b"not-wasm";
        let manifest = gang_core::manifest::ComponentManifest {
            schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
            name: "needs-spawn".into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![CapabilityGroup::ProcessSpawn {
                version: "1.0".into(),
                allowed_commands: vec!["ls".into()],
            }],
            author_peer_id: kp.peer_id(),
            component_hash: blake3::hash(component).to_hex().to_string(),
            limits: Default::default(),
            language: Default::default(),
            description: String::new(),
            tags: vec![],
            min_ganglion_version: None,
        };
        let signed = SignedManifest::sign(&manifest, &kp).unwrap();
        let manifest_cbor = signed.to_cbor().unwrap();

        let result = agent
            .deploy_capability(&manifest_cbor, component, &kp.peer_id())
            .await;
        assert!(result.is_err(), "process/spawn must be denied by policy");

        // The feed carries a PolicyDecision{Deny} for the denied deploy.
        let batch = agent
            .build_event_subscription(&kp.peer_id(), &EventSubscribeRequest::fresh())
            .await
            .unwrap();
        let denied = batch.iter().any(|e| {
            matches!(
                e,
                AgentEvent::PolicyDecision {
                    decision: PolicyOutcome::Deny,
                    ..
                }
            )
        });
        assert!(
            denied,
            "a PolicyDecision Deny should be on the feed: {batch:?}"
        );
    }

    #[tokio::test]
    async fn allowed_invoke_emits_audit_appended_on_feed() {
        let dir = TempDir::new().unwrap();
        let agent = RobotAgent::new(test_config(dir.path())).unwrap();
        let kp = Keypair::generate();

        // Deploy + invoke a diagnostics capability via the broker path.
        let component = b"fake wasm bytes for testing";
        let manifest = gang_core::manifest::ComponentManifest {
            schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
            name: "diag".into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            }],
            author_peer_id: kp.peer_id(),
            component_hash: blake3::hash(component).to_hex().to_string(),
            limits: Default::default(),
            language: Default::default(),
            description: String::new(),
            tags: vec![],
            min_ganglion_version: None,
        };
        let signed = SignedManifest::sign(&manifest, &kp).unwrap();
        agent
            .deploy_capability(&signed.to_cbor().unwrap(), component, &kp.peer_id())
            .await
            .unwrap();
        agent
            .invoke_capability("diag", &[], &kp.peer_id())
            .await
            .unwrap();

        let batch = agent
            .build_event_subscription(&kp.peer_id(), &EventSubscribeRequest::fresh())
            .await
            .unwrap();
        assert!(
            batch
                .iter()
                .any(|e| matches!(e, AgentEvent::AuditAppended { .. })),
            "an AuditAppended event should be on the feed: {batch:?}"
        );
        assert!(
            batch.iter().any(|e| matches!(
                e,
                AgentEvent::PolicyDecision {
                    decision: PolicyOutcome::Allow,
                    ..
                }
            )),
            "a PolicyDecision Allow (from deploy) should be on the feed"
        );
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

    /// Deploy a capability whose component bytes begin with the WASM magic
    /// header (so the invoke path treats them as a WASM component) but are not
    /// a valid component. Used to drive the tamper / trap tests.
    fn wasmish_bytes(tail: &[u8]) -> Vec<u8> {
        let mut b = vec![0x00, b'a', b's', b'm', 0x01, 0x00, 0x00, 0x00];
        b.extend_from_slice(tail);
        b
    }

    async fn deploy_wasmish(agent: &RobotAgent, kp: &Keypair, name: &str, bytes: &[u8]) -> String {
        let manifest = gang_core::manifest::ComponentManifest {
            schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
            name: name.into(),
            version: "0.1.0".into(),
            declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            }],
            author_peer_id: kp.peer_id(),
            component_hash: blake3::hash(bytes).to_hex().to_string(),
            limits: gang_core::manifest::ResourceLimits::default(),
            language: Default::default(),
            description: String::new(),
            tags: vec![],
            min_ganglion_version: None,
        };
        let signed = SignedManifest::sign(&manifest, kp).unwrap();
        let manifest_cbor = signed.to_cbor().unwrap();
        agent
            .deploy_capability(&manifest_cbor, bytes, &kp.peer_id())
            .await
            .unwrap()
    }

    // --- SEC-01: fail-closed policy / trust-store loading ---

    #[tokio::test]
    async fn malformed_policy_fails_closed() {
        let dir = TempDir::new().unwrap();
        let policy_path = dir.path().join("policy.toml");
        // Not valid TOML.
        std::fs::write(&policy_path, "this is : not = valid : toml").unwrap();
        let mut config = test_config(dir.path());
        config.policy_path = Some(policy_path);
        let result = RobotAgent::new(config);
        assert!(
            result.is_err(),
            "a malformed policy must fail closed, not fall back to permissive"
        );
    }

    #[tokio::test]
    async fn malformed_trust_store_fails_closed() {
        let dir = TempDir::new().unwrap();
        let trust_path = dir.path().join("trusted_peers.json");
        std::fs::write(&trust_path, "{ not valid json").unwrap();
        let mut config = test_config(dir.path());
        config.trust_store_path = trust_path;
        let result = RobotAgent::new(config);
        assert!(
            result.is_err(),
            "a malformed trust store must fail closed, not start empty"
        );
    }

    #[tokio::test]
    async fn absent_policy_is_permissive_dev_path() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path());
        config.policy_path = None; // dev path
        let agent = RobotAgent::new(config).unwrap();
        // Permissive policy allows any declared capability from any peer.
        let kp = Keypair::generate();
        assert!(
            agent
                .policy
                .evaluate(
                    &[CapabilityGroup::DiagnosticsCollect {
                        version: "1.0".into(),
                    }],
                    &kp.peer_id(),
                )
                .is_ok()
        );
    }

    // --- SEC-06: WASM path uses the agent's configured brokers ---

    #[tokio::test]
    async fn wasm_fs_broker_honors_configured_restriction() {
        use gang_core::broker::{BrokerOperation, CapabilityRequest};

        let dir = TempDir::new().unwrap();
        let allowed = dir.path().join("allowed");
        std::fs::create_dir_all(&allowed).unwrap();

        let mut config = test_config(dir.path());
        // Restrict fs access to `allowed/` only — NOT the old permissive
        // /tmp/gang/** default.
        config.fs_allowed_patterns = vec![FsRule {
            pattern: format!("{}/**", allowed.display()),
            read: true,
            write: true,
        }];
        let agent = RobotAgent::new(config).unwrap();

        // The broker the WASM runtime would route fs calls through is the
        // agent's configured instance.
        let fs = agent
            .runtime
            .broker("ganglion:fs/bounded")
            .expect("fs broker registered on runtime");

        // A write inside the allowed directory succeeds.
        let ok = fs
            .handle_request(CapabilityRequest {
                capability_group: "ganglion:fs/bounded".into(),
                operation: BrokerOperation::FsWrite {
                    path: allowed.join("ok.txt").display().to_string(),
                    data: b"hi".to_vec(),
                },
            })
            .await;
        assert!(ok.is_ok(), "write inside configured pattern should succeed");

        // A write to /tmp/gang (the OLD permissive default) is denied.
        let denied = fs
            .handle_request(CapabilityRequest {
                capability_group: "ganglion:fs/bounded".into(),
                operation: BrokerOperation::FsWrite {
                    path: "/tmp/gang/evil.txt".into(),
                    data: b"x".to_vec(),
                },
            })
            .await;
        assert!(
            denied.is_err(),
            "write outside configured pattern must be denied"
        );
    }

    // --- SEC-07: tampered component rejected before execution ---

    #[tokio::test]
    async fn tampered_component_rejected_on_invoke() {
        let dir = TempDir::new().unwrap();
        let agent = RobotAgent::new(test_config(dir.path())).unwrap();
        let kp = Keypair::generate();

        let original = wasmish_bytes(b"original-body");
        let name = deploy_wasmish(&agent, &kp, "tamper-test", &original).await;

        // Tamper the on-disk component after install (still WASM-magic so the
        // invoke path treats it as a component).
        let wasm_path = dir
            .path()
            .join("capabilities")
            .join(&name)
            .join(format!("{name}.wasm"));
        std::fs::write(&wasm_path, wasmish_bytes(b"TAMPERED-body")).unwrap();

        let result = agent.invoke_capability(&name, &[], &kp.peer_id()).await;
        assert!(result.is_err(), "tampered component must be rejected");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .to_lowercase()
                .contains("hash"),
            "error should mention the hash mismatch"
        );

        // Audit log records a non-success status for the rejected invocation.
        let records = agent.audit_log.read_all().unwrap();
        let last = records.last().expect("an audit record was written");
        assert!(!matches!(last.exit_status, ExitStatus::Success));
    }

    // --- SEC-13: WASM failure is terminal + audited, no broker fallback ---

    #[tokio::test]
    async fn trapping_component_is_terminal_and_audited() {
        let dir = TempDir::new().unwrap();
        let agent = RobotAgent::new(test_config(dir.path())).unwrap();
        let kp = Keypair::generate();

        // WASM-magic bytes that are NOT a valid component: compilation fails
        // inside the runtime. The capability declares DiagnosticsCollect, so a
        // silent fallback to the ambient diagnostics broker would have produced
        // a successful `system_info` result — we assert that does NOT happen.
        let bytes = wasmish_bytes(b"not-a-real-component");
        let name = deploy_wasmish(&agent, &kp, "trap-test", &bytes).await;

        let result = agent.invoke_capability(&name, &[], &kp.peer_id()).await;
        assert!(
            result.is_err(),
            "a failing WASM run must be a terminal error, not fall back to brokers"
        );

        let records = agent.audit_log.read_all().unwrap();
        let last = records.last().expect("an audit record was written");
        assert!(
            !matches!(last.exit_status, ExitStatus::Success),
            "audit must record the real failure status, got {:?}",
            last.exit_status
        );
    }
}
