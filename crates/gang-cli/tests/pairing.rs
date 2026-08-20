//! In-process integration tests for one-line enrollment (`gang pair` / `gang
//! join`, issue #5).
//!
//! These assemble the real roles on loopback — a circuit-relay v2 server, an
//! operator that reserves a circuit and serves the enrollment handler, and a
//! robot (`RobotAgent` + transport) that dials out and enrolls — and drive the
//! same control-message exchange the CLI performs. The operator's enrollment
//! decision is the shipping one: `gang_core::pairing::authorize_enrollment`.
//!
//! The trust-critical assertions are:
//!   * the operator records the robot under the id libp2p **authenticated on the
//!     wire** (`GanglionStream::remote_peer`), not a self-report;
//!   * a robot cannot enroll as a dialable id whose key it does not hold
//!     (identity-mismatch rejection);
//!   * a pairing token is single-use (reuse rejected) and expiring (expired
//!     token rejected);
//!   * after pairing, the operator can actually reach the robot (a real deploy
//!     over the relay circuit succeeds).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gang_core::capability::CapabilityGroup;
use gang_core::identity::{Keypair, PeerId, PeerRegistry};
use gang_core::manifest::{ComponentManifest, ResourceLimits, SignedManifest, TrustStore};
use gang_core::message::{
    ControlMessage, InvokeStatus, decode_message, encode_message, fresh_nonce, unix_millis_now,
};
use gang_core::pairing::{DEFAULT_TTL, PairingToken, authorize_enrollment};
use gang_core::protocol::ProtocolId;
use gang_core::transport::{StreamHandler, TransportAdapter};
use gang_libp2p::{Libp2pConfig, Libp2pTransportAdapter};
use gang_ros::agent::{AgentConfig, RobotAgent};
use gang_ros::filesystem::FsRule;
use tempfile::TempDir;

const WAIT_SECS: u64 = 30;

async fn wait_until<F>(what: &str, mut cond: F)
where
    F: AsyncFnMut() -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(WAIT_SECS);
    loop {
        if cond().await {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out after {WAIT_SECS}s waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn start_adapter(config: Libp2pConfig) -> Arc<Libp2pTransportAdapter> {
    let adapter = Arc::new(
        Libp2pTransportAdapter::new(config)
            .await
            .expect("adapter builds"),
    );
    let looped = Arc::clone(&adapter);
    tokio::spawn(async move {
        let _ = looped.run_event_loop().await;
    });
    adapter
}

async fn start_relay(dir: &Path) -> (Arc<Libp2pTransportAdapter>, String) {
    let relay = start_adapter(Libp2pConfig {
        key_path: dir.join("relay.key"),
        listen_addrs: vec!["/ip4/127.0.0.1/tcp/0".into()],
        relay_server: true,
        ..Default::default()
    })
    .await;

    let mut tcp = String::new();
    wait_until("relay listen address", async || {
        if let Some(a) = relay
            .listen_addrs()
            .await
            .iter()
            .find(|a| a.contains("/tcp/"))
        {
            tcp = a.clone();
            true
        } else {
            false
        }
    })
    .await;
    let dial = format!("{tcp}/p2p/{}", relay.libp2p_peer_id());
    (relay, dial)
}

/// Wait for a circuit reservation to appear as a listen address.
async fn wait_for_reservation(t: &Arc<Libp2pTransportAdapter>) {
    wait_until("relay circuit reservation", async || {
        t.listen_addrs()
            .await
            .iter()
            .any(|a| a.contains("p2p-circuit"))
    })
    .await;
}

/// Dial `dest_libp2p` through `relay` and wait until `dest_gang` is a connected,
/// authenticated peer — the point at which the wire has proved the far identity.
async fn connect_via_circuit(
    from: &Arc<Libp2pTransportAdapter>,
    relay: &str,
    dest_gang: &PeerId,
    dest_libp2p: &str,
) {
    from.dial_multiaddr(relay).await.expect("relay dial queued");
    wait_until("connection to relay", async || {
        !from.connected_peers().await.is_empty()
    })
    .await;
    let circuit = format!("{relay}/p2p-circuit/p2p/{dest_libp2p}");
    from.dial_multiaddr(&circuit)
        .await
        .expect("circuit dial queued");
    wait_until("authenticated connection to destination", async || {
        from.connected_peers()
            .await
            .iter()
            .any(|(id, _)| id == dest_gang)
    })
    .await;
}

/// The operator's enrollment handler, sharing the shipping trust decision with
/// the CLI (`authorize_enrollment`). Records into `registry_path`/`trust_path`
/// on success. `consumed` enforces single-use.
fn enroll_handler(
    token: Arc<PairingToken>,
    consumed: Arc<AtomicBool>,
    operator_id: PeerId,
    registry_path: std::path::PathBuf,
    trust_path: std::path::PathBuf,
) -> StreamHandler {
    Box::new(move |mut stream| {
        let token = Arc::clone(&token);
        let consumed = Arc::clone(&consumed);
        let operator_id = operator_id.clone();
        let registry_path = registry_path.clone();
        let trust_path = trust_path.clone();
        Box::pin(async move {
            use gang_core::identity::{PeerEntry, Role};
            use gang_core::manifest::TrustedPeer;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let wire_gang_id = stream.remote_peer.clone();
            let mut buf = Vec::new();
            let _ = stream.inner.read_to_end(&mut buf).await;
            let (secret, name, libp2p_id) = match decode_message::<ControlMessage>(&buf) {
                Ok((
                    ControlMessage::Enroll {
                        token_secret,
                        name,
                        libp2p_id,
                    },
                    _,
                )) => (token_secret, name, libp2p_id),
                _ => return,
            };

            let err = |m: &str| ControlMessage::Error {
                request_id: None,
                code: "rejected".into(),
                message: m.into(),
            };

            let ident = gang_libp2p::identity_from_libp2p_str(&libp2p_id);
            let now = unix_millis_now();
            let response = match ident {
                None => err("bad id"),
                Some(id) => {
                    match authorize_enrollment(&token, &secret, &wire_gang_id, &id.gang_id, now) {
                        Err(e) => err(&e.to_string()),
                        Ok(()) => {
                            if consumed.swap(true, Ordering::SeqCst) {
                                err("pairing token already used")
                            } else {
                                let mut reg = PeerRegistry::load(&registry_path).unwrap();
                                reg.register(
                                    name.clone(),
                                    PeerEntry {
                                        peer_id: wire_gang_id.clone(),
                                        role: Role::RobotAgent,
                                        relay_addrs: vec![token.relay_addr.clone()],
                                        libp2p_id: Some(id.libp2p_id.clone()),
                                    },
                                );
                                reg.save(&registry_path).unwrap();
                                let mut trust = TrustStore::load(&trust_path).unwrap();
                                trust.add(TrustedPeer {
                                    peer_id: wire_gang_id.clone(),
                                    name: name.clone(),
                                    public_key: id.ed25519_pubkey.to_vec(),
                                });
                                trust.save(&trust_path).unwrap();
                                ControlMessage::Enrolled {
                                    operator_id: operator_id.clone(),
                                    robot_id: wire_gang_id.clone(),
                                    name,
                                }
                            }
                        }
                    }
                }
            };
            if let Ok(b) = encode_message(&response) {
                let _ = stream.inner.write_all(&b).await;
            }
        })
    })
}

/// Stand up relay + operator (reserving a circuit and serving the enroll
/// handler). Returns everything the robot side needs.
struct OperatorFixture {
    _relay: Arc<Libp2pTransportAdapter>,
    relay_addr: String,
    operator: Arc<Libp2pTransportAdapter>,
    operator_id: PeerId,
    operator_libp2p: String,
    token: Arc<PairingToken>,
    consumed: Arc<AtomicBool>,
    registry_path: std::path::PathBuf,
}

async fn setup_operator(dir: &Path, ttl_from_now: Duration) -> OperatorFixture {
    let (relay, relay_addr) = start_relay(dir).await;

    let op_key = dir.join("operator.key");
    let operator = start_adapter(Libp2pConfig {
        key_path: op_key.clone(),
        listen_addrs: vec![],
        relay_addrs: vec![relay_addr.clone()],
        ..Default::default()
    })
    .await;
    let operator_id = Keypair::load(&op_key).unwrap().peer_id();
    let operator_libp2p = operator.libp2p_peer_id().to_string();
    wait_for_reservation(&operator).await;

    let now = unix_millis_now();
    let token = Arc::new(PairingToken::mint(
        &relay_addr,
        &operator_libp2p,
        now,
        ttl_from_now,
    ));
    let consumed = Arc::new(AtomicBool::new(false));
    let registry_path = dir.join("peers.json");
    let trust_path = dir.join("trusted_peers.json");

    operator
        .listen(
            ProtocolId::control(),
            enroll_handler(
                Arc::clone(&token),
                Arc::clone(&consumed),
                operator_id.clone(),
                registry_path.clone(),
                trust_path.clone(),
            ),
        )
        .await
        .expect("operator registers enroll handler");

    OperatorFixture {
        _relay: relay,
        relay_addr,
        operator,
        operator_id,
        operator_libp2p,
        token,
        consumed,
        registry_path,
    }
}

fn agent_config(dir: &Path) -> AgentConfig {
    AgentConfig {
        key_path: dir.join("identity.key"),
        policy_path: None,
        trust_store_path: dir.join("trusted_peers.json"),
        capabilities_dir: dir.join("capabilities"),
        audit_log_path: dir.join("audit.log"),
        audit_max_size_bytes: 10 * 1024 * 1024,
        policy_resync_interval_secs: 0,
        credentials_path: None,
        usage_bundle_path: None,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    }
}

/// Start a robot with an agent serving control, and reserve a circuit.
async fn start_robot(
    dir: &Path,
    relay_addr: &str,
    operator_id: &PeerId,
    operator_pubkey: &[u8],
) -> (Arc<RobotAgent>, Arc<Libp2pTransportAdapter>) {
    // Trust the operator so its later deploys are authorized (SEC-03).
    let mut trust = TrustStore::default();
    trust.add(gang_core::manifest::TrustedPeer {
        peer_id: operator_id.clone(),
        name: "operator".into(),
        public_key: operator_pubkey.to_vec(),
    });
    trust.save(&dir.join("trusted_peers.json")).unwrap();

    let agent = Arc::new(RobotAgent::new(agent_config(dir)).expect("agent builds"));
    let transport = start_adapter(Libp2pConfig {
        key_path: dir.join("identity.key"),
        listen_addrs: vec![],
        relay_addrs: vec![relay_addr.to_string()],
        ..Default::default()
    })
    .await;
    agent.serve(transport.as_ref()).await.expect("agent serves");
    wait_for_reservation(&transport).await;
    (agent, transport)
}

/// Send an Enroll and return the operator's decoded response.
async fn enroll(
    robot: &Arc<Libp2pTransportAdapter>,
    operator_id: &PeerId,
    secret: Vec<u8>,
    name: &str,
    libp2p_id: &str,
) -> ControlMessage {
    let req = ControlMessage::Enroll {
        token_secret: secret,
        name: name.into(),
        libp2p_id: libp2p_id.into(),
    };
    let bytes = encode_message(&req).unwrap();
    let resp = robot
        .send_rpc_with_timeout(operator_id, bytes, Duration::from_secs(WAIT_SECS))
        .await
        .expect("enroll rpc");
    decode_message::<ControlMessage>(&resp).unwrap().0
}

#[tokio::test]
async fn pairing_happy_path_records_wire_authenticated_id_and_enables_deploy() {
    let dir = TempDir::new().unwrap();
    let op = setup_operator(dir.path(), DEFAULT_TTL).await;
    let operator_pubkey = Keypair::load(&dir.path().join("operator.key"))
        .unwrap()
        .public_key()
        .to_bytes()
        .to_vec();

    let robot_dir = dir.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();
    let (robot_agent, robot) = start_robot(
        &robot_dir,
        &op.relay_addr,
        &op.operator_id,
        &operator_pubkey,
    )
    .await;
    let robot_gang = robot_agent.peer_id().clone();
    let robot_libp2p = robot.libp2p_peer_id().to_string();

    // Robot dials the operator through the relay and enrolls.
    connect_via_circuit(&robot, &op.relay_addr, &op.operator_id, &op.operator_libp2p).await;
    let resp = enroll(
        &robot,
        &op.operator_id,
        op.token.secret.to_vec(),
        "field-01",
        &robot_libp2p,
    )
    .await;

    match resp {
        ControlMessage::Enrolled {
            robot_id,
            name,
            operator_id,
        } => {
            assert_eq!(name, "field-01");
            assert_eq!(operator_id, op.operator_id);
            // The recorded id is exactly the one authenticated on the wire.
            assert_eq!(robot_id, robot_gang);
        }
        other => panic!("expected Enrolled, got {other:?}"),
    }

    // The operator's registry now lists the robot by its wire-authenticated id.
    let reg = PeerRegistry::load(&op.registry_path).unwrap();
    let entry = reg.lookup("field-01").expect("robot registered");
    assert_eq!(entry.peer_id, robot_gang);
    assert_eq!(entry.libp2p_id.as_deref(), Some(robot_libp2p.as_str()));

    // And the operator can actually reach it: a real deploy over the circuit.
    let op_key = Keypair::load(&dir.path().join("operator.key")).unwrap();
    let component = b"pairing-e2e-diagnostics".to_vec();
    let manifest = ComponentManifest {
        schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
        name: "diagnostics".into(),
        version: "0.1.0".into(),
        declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }],
        author_peer_id: op_key.peer_id(),
        component_hash: blake3::hash(&component).to_hex().to_string(),
        limits: ResourceLimits::default(),
        language: Default::default(),
        description: String::new(),
        tags: vec![],
        min_ganglion_version: None,
        credential_slots: vec![],
        exports: vec![],
    };
    let manifest_cbor = SignedManifest::sign(&manifest, &op_key)
        .unwrap()
        .to_cbor()
        .unwrap();
    let deploy = ControlMessage::DeployCapability {
        name: "diagnostics".into(),
        version: "0.1.0".into(),
        manifest_cbor,
        component_bytes: component,
        nonce: fresh_nonce(),
        timestamp_ms: unix_millis_now(),
    };
    let bytes = encode_message(&deploy).unwrap();
    let resp = op
        .operator
        .send_rpc_with_timeout(&robot_gang, bytes, Duration::from_secs(WAIT_SECS))
        .await
        .expect("deploy rpc");
    match decode_message::<ControlMessage>(&resp).unwrap().0 {
        ControlMessage::InvokeResult { status, .. } => {
            assert!(
                matches!(status, InvokeStatus::Success),
                "deploy should succeed"
            );
        }
        other => panic!("expected InvokeResult, got {other:?}"),
    }
}

#[tokio::test]
async fn pairing_rejects_reused_token() {
    let dir = TempDir::new().unwrap();
    let op = setup_operator(dir.path(), DEFAULT_TTL).await;
    let operator_pubkey = Keypair::load(&dir.path().join("operator.key"))
        .unwrap()
        .public_key()
        .to_bytes()
        .to_vec();

    let robot_dir = dir.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();
    let (_agent, robot) = start_robot(
        &robot_dir,
        &op.relay_addr,
        &op.operator_id,
        &operator_pubkey,
    )
    .await;
    let robot_libp2p = robot.libp2p_peer_id().to_string();
    connect_via_circuit(&robot, &op.relay_addr, &op.operator_id, &op.operator_libp2p).await;

    let first = enroll(
        &robot,
        &op.operator_id,
        op.token.secret.to_vec(),
        "r",
        &robot_libp2p,
    )
    .await;
    assert!(
        matches!(first, ControlMessage::Enrolled { .. }),
        "first enroll succeeds"
    );
    assert!(op.consumed.load(Ordering::SeqCst), "token consumed");

    let second = enroll(
        &robot,
        &op.operator_id,
        op.token.secret.to_vec(),
        "r",
        &robot_libp2p,
    )
    .await;
    match second {
        ControlMessage::Error { message, .. } => assert!(
            message.contains("already used"),
            "reuse must be rejected, got: {message}"
        ),
        other => panic!("expected Error on reuse, got {other:?}"),
    }
}

#[tokio::test]
async fn pairing_rejects_identity_it_does_not_hold() {
    let dir = TempDir::new().unwrap();
    let op = setup_operator(dir.path(), DEFAULT_TTL).await;
    let operator_pubkey = Keypair::load(&dir.path().join("operator.key"))
        .unwrap()
        .public_key()
        .to_bytes()
        .to_vec();

    let robot_dir = dir.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();
    let (_agent, robot) = start_robot(
        &robot_dir,
        &op.relay_addr,
        &op.operator_id,
        &operator_pubkey,
    )
    .await;
    connect_via_circuit(&robot, &op.relay_addr, &op.operator_id, &op.operator_libp2p).await;

    // The robot claims a DIFFERENT peer's dialable id — one whose key it does
    // not hold. libp2p still authenticates the robot's real key on the wire, so
    // the derived id will not match and the operator must reject. A throwaway
    // adapter gives a genuine, distinct dialable id.
    let impostor = Libp2pTransportAdapter::new(Libp2pConfig {
        key_path: dir.path().join("impostor.key"),
        listen_addrs: vec![],
        ..Default::default()
    })
    .await
    .expect("impostor adapter");
    let impostor_libp2p = impostor.libp2p_peer_id().to_string();

    let resp = enroll(
        &robot,
        &op.operator_id,
        op.token.secret.to_vec(),
        "impostor",
        &impostor_libp2p,
    )
    .await;
    match resp {
        ControlMessage::Error { message, .. } => assert!(
            message.contains("wire-authenticated"),
            "identity mismatch must be rejected, got: {message}"
        ),
        other => panic!("expected Error on identity mismatch, got {other:?}"),
    }
    assert!(
        !op.consumed.load(Ordering::SeqCst),
        "a rejected enroll must not consume the token"
    );
}

#[tokio::test]
async fn pairing_rejects_expired_token() {
    let dir = TempDir::new().unwrap();
    // Mint a token that is already expired.
    let op = setup_operator(dir.path(), Duration::from_secs(0)).await;
    // Give wall-clock a moment so now > expires_at.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let operator_pubkey = Keypair::load(&dir.path().join("operator.key"))
        .unwrap()
        .public_key()
        .to_bytes()
        .to_vec();

    let robot_dir = dir.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();
    let (_agent, robot) = start_robot(
        &robot_dir,
        &op.relay_addr,
        &op.operator_id,
        &operator_pubkey,
    )
    .await;
    let robot_libp2p = robot.libp2p_peer_id().to_string();
    connect_via_circuit(&robot, &op.relay_addr, &op.operator_id, &op.operator_libp2p).await;

    let resp = enroll(
        &robot,
        &op.operator_id,
        op.token.secret.to_vec(),
        "r",
        &robot_libp2p,
    )
    .await;
    match resp {
        ControlMessage::Error { message, .. } => {
            assert!(
                message.contains("expired"),
                "expired token must be rejected, got: {message}"
            );
        }
        other => panic!("expected Error on expiry, got {other:?}"),
    }
}
