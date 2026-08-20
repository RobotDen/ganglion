//! In-process integration tests for ADR-020 Phase 32 remote dispatch.
//!
//! Each test assembles the three real roles on loopback — a circuit-relay v2
//! server, a robot (`RobotAgent` + transport, serving `/ganglion/control/1.0`),
//! and an operator transport — and drives Deploy → Invoke → List through the
//! same control-message exchange the `gang` CLI performs.
//!
//! The primary tests dial the robot **through the relay circuit**
//! (`<relay>/p2p-circuit/p2p/<robot>`), which is the product claim: the robot
//! binds no direct listen address at all, so the relay path is the only way
//! in. A direct-dial variant covers the flat-network case as well.
//!
//! All listeners use ephemeral loopback ports, so tests can run concurrently
//! without port clashes.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gang_core::capability::CapabilityGroup;
use gang_core::identity::{Keypair, PeerId};
use gang_core::manifest::{
    ComponentManifest, ResourceLimits, SignedManifest, TrustStore, TrustedPeer,
};
use gang_core::message::{
    ControlMessage, InvokeStatus, decode_message, encode_message, fresh_nonce, unix_millis_now,
};
use gang_libp2p::{Libp2pConfig, Libp2pTransportAdapter};
use gang_ros::agent::{AgentConfig, RobotAgent};
use gang_ros::filesystem::FsRule;

/// Bound every network wait; loopback circuits establish in well under this.
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

/// Build an adapter and spawn its event loop.
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

/// Start a relay server on an ephemeral loopback TCP port and return it with
/// its dialable multiaddr (`/ip4/127.0.0.1/tcp/<port>/p2p/<libp2p-id>`).
async fn start_relay(dir: &Path) -> (Arc<Libp2pTransportAdapter>, String) {
    let relay = start_adapter(Libp2pConfig {
        key_path: dir.join("relay.key"),
        listen_addrs: vec!["/ip4/127.0.0.1/tcp/0".into()],
        relay_server: true,
        ..Default::default()
    })
    .await;

    let mut tcp_addr = String::new();
    wait_until("relay to report its resolved listen address", async || {
        if let Some(a) = relay
            .listen_addrs()
            .await
            .iter()
            .find(|a| a.contains("/tcp/"))
        {
            tcp_addr = a.clone();
            true
        } else {
            false
        }
    })
    .await;

    let dial_addr = format!("{tcp_addr}/p2p/{}", relay.libp2p_peer_id());
    (relay, dial_addr)
}

fn agent_config(dir: &Path) -> AgentConfig {
    AgentConfig {
        key_path: dir.join("identity.key"),
        policy_path: None, // permissive dev policy
        trust_store_path: dir.join("trusted_peers.json"),
        capabilities_dir: dir.join("capabilities"),
        audit_log_path: dir.join("audit.log"),
        audit_max_size_bytes: 10 * 1024 * 1024,
        policy_resync_interval_secs: 0,
        credentials_path: None,
        usage_bundle_path: Some(dir.join("usage-bundle.json")),
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    }
}

/// Start a robot: `RobotAgent` + transport serving the control protocol.
///
/// When `relay_dial_addr` is set the robot binds NO direct listen address and
/// requests a circuit reservation instead — reachable through the relay only.
async fn start_robot(
    dir: &Path,
    relay_dial_addr: Option<&str>,
) -> (Arc<RobotAgent>, Arc<Libp2pTransportAdapter>) {
    let config = agent_config(dir);
    let agent = Arc::new(RobotAgent::new(config).expect("agent builds"));

    let (listen_addrs, relay_addrs) = match relay_dial_addr {
        Some(relay) => (vec![], vec![relay.to_string()]),
        None => (vec!["/ip4/127.0.0.1/tcp/0".into()], vec![]),
    };
    let transport = start_adapter(Libp2pConfig {
        key_path: dir.join("identity.key"),
        listen_addrs,
        relay_addrs,
        ..Default::default()
    })
    .await;

    agent
        .serve(transport.as_ref())
        .await
        .expect("agent serves control protocol");

    if relay_dial_addr.is_some() {
        // The robot is reachable once the relay accepts its reservation,
        // observable as a circuit listen address.
        wait_until("robot's relay circuit reservation", async || {
            transport
                .listen_addrs()
                .await
                .iter()
                .any(|a| a.contains("p2p-circuit"))
        })
        .await;
    }

    (agent, transport)
}

/// Start an operator transport (outbound-only) and connect it to the robot
/// through the relay circuit. Returns the operator adapter, its signing
/// keypair, and whether the robot connection was relayed.
async fn connect_operator_via_circuit(
    dir: &Path,
    relay_dial_addr: &str,
    robot: &Arc<Libp2pTransportAdapter>,
) -> (Arc<Libp2pTransportAdapter>, Keypair, bool) {
    let key_path = dir.join("identity.key");
    let operator = start_adapter(Libp2pConfig {
        key_path: key_path.clone(),
        listen_addrs: vec![],
        ..Default::default()
    })
    .await;
    let keypair = Keypair::load(&key_path).expect("operator key persisted");

    operator
        .dial_multiaddr(relay_dial_addr)
        .await
        .expect("relay dial queued");

    // Wait for the relay connection before requesting the circuit, mirroring
    // the CLI dispatch path (a circuit dial races an in-flight relay dial).
    wait_until("operator connection to the relay", async || {
        !operator.connected_peers().await.is_empty()
    })
    .await;

    let circuit = format!(
        "{relay_dial_addr}/p2p-circuit/p2p/{}",
        robot.libp2p_peer_id()
    );
    operator
        .dial_multiaddr(&circuit)
        .await
        .expect("circuit dial queued");

    let robot_gang_id = robot.local_peer_id_for_test();
    let mut via_relay = false;
    wait_until(
        "operator connection to the robot via circuit",
        async || match operator
            .connected_peers()
            .await
            .iter()
            .find(|(id, _)| *id == robot_gang_id)
        {
            Some((_, relayed)) => {
                via_relay = *relayed;
                true
            }
            None => false,
        },
    )
    .await;

    (operator, keypair, via_relay)
}

/// Local test extension: the gang peer id of an adapter.
trait GangId {
    fn local_peer_id_for_test(&self) -> PeerId;
}
impl GangId for Libp2pTransportAdapter {
    fn local_peer_id_for_test(&self) -> PeerId {
        use gang_core::transport::TransportAdapter;
        self.local_peer_id()
    }
}

/// A signed manifest for placeholder (non-WASM) component bytes; the agent
/// serves these through the direct broker path, which is enough to prove the
/// dispatch plumbing without a wasm toolchain in CI.
fn signed_manifest_cbor(author: &Keypair, name: &str, component: &[u8]) -> Vec<u8> {
    let manifest = ComponentManifest {
        schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
        name: name.into(),
        version: "0.1.0".into(),
        declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }],
        author_peer_id: author.peer_id(),
        component_hash: blake3_hex(component),
        limits: ResourceLimits::default(),
        language: Default::default(),
        description: String::new(),
        tags: vec![],
        min_ganglion_version: None,
        credential_slots: vec![],
        exports: vec![],
    };
    SignedManifest::sign(&manifest, author)
        .expect("manifest signs")
        .to_cbor()
        .expect("manifest encodes")
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn deploy_message(author: &Keypair, name: &str, component: &[u8]) -> ControlMessage {
    ControlMessage::DeployCapability {
        name: name.into(),
        version: "0.1.0".into(),
        manifest_cbor: signed_manifest_cbor(author, name, component),
        component_bytes: component.to_vec(),
        nonce: fresh_nonce(),
        timestamp_ms: unix_millis_now(),
    }
}

fn invoke_message(name: &str) -> ControlMessage {
    ControlMessage::InvokeCapability {
        name: name.into(),
        args: vec![],
        request_id: fresh_nonce(),
        nonce: fresh_nonce(),
        export: None,
        timestamp_ms: unix_millis_now(),
    }
}

/// Send a control message and decode the framed response.
async fn rpc(
    operator: &Libp2pTransportAdapter,
    robot: &PeerId,
    message: &ControlMessage,
) -> ControlMessage {
    let bytes = encode_message(message).expect("request encodes");
    rpc_raw(operator, robot, bytes).await
}

async fn rpc_raw(
    operator: &Libp2pTransportAdapter,
    robot: &PeerId,
    bytes: Vec<u8>,
) -> ControlMessage {
    let response = operator
        .send_rpc_with_timeout(robot, bytes, Duration::from_secs(WAIT_SECS))
        .await
        .expect("rpc completes");
    assert!(!response.is_empty(), "robot sent an empty response");
    decode_message::<ControlMessage>(&response)
        .expect("response decodes")
        .0
}

/// Deploy → Invoke → List through the relay circuit, plus replay rejection.
#[tokio::test(flavor = "multi_thread")]
async fn circuit_dispatch_deploy_invoke_list_and_replay() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;
    let (_agent, robot_transport) =
        start_robot(&dirs.path().join("robot"), Some(&relay_addr)).await;
    let (operator, operator_kp, via_relay) =
        connect_operator_via_circuit(&dirs.path().join("operator"), &relay_addr, &robot_transport)
            .await;

    // The robot binds no direct listener, so this connection can only have
    // come through the relay circuit.
    assert!(via_relay, "connection to the robot must be relayed");

    let robot_id = robot_transport.local_peer_id_for_test();
    let component = b"gang-test-component (not wasm; broker path)".to_vec();

    // Deploy.
    let response = rpc(
        &operator,
        &robot_id,
        &deploy_message(&operator_kp, "circuit-diag", &component),
    )
    .await;
    match response {
        ControlMessage::InvokeResult { status, output, .. } => {
            assert!(matches!(status, InvokeStatus::Success), "deploy failed");
            assert_eq!(String::from_utf8_lossy(&output), "circuit-diag");
        }
        other => panic!("unexpected deploy response: {other:?}"),
    }

    // Invoke — a real broker-backed result must come back over the circuit.
    let invoke = invoke_message("circuit-diag");
    let invoke_bytes = encode_message(&invoke).expect("request encodes");
    let response = rpc_raw(&operator, &robot_id, invoke_bytes.clone()).await;
    match response {
        ControlMessage::InvokeResult { status, output, .. } => {
            assert!(matches!(status, InvokeStatus::Success), "invoke failed");
            let value: serde_json::Value =
                serde_json::from_slice(&output).expect("invoke output is JSON");
            assert!(
                value.get("system_info").is_some(),
                "diagnostics output missing system_info: {value}"
            );
        }
        other => panic!("unexpected invoke response: {other:?}"),
    }

    // Replaying the captured request bytes must be rejected (SEC-14).
    let response = rpc_raw(&operator, &robot_id, invoke_bytes).await;
    match response {
        ControlMessage::Error { code, .. } => assert_eq!(code, "replay_rejected"),
        other => panic!("replayed request was not rejected: {other:?}"),
    }

    // List capabilities.
    let response = rpc(&operator, &robot_id, &ControlMessage::ListCapabilities).await;
    match response {
        ControlMessage::CapabilityList { capabilities } => {
            assert!(
                capabilities.iter().any(|c| c.name == "circuit-diag"),
                "deployed capability missing from list: {capabilities:?}"
            );
        }
        other => panic!("unexpected list response: {other:?}"),
    }

    // Fetch the usage bundle over the same circuit (ADR-027): the invoke
    // above must appear as one diagnostics-group success; the capability
    // name must not appear anywhere; the fetch resets robot-side counters.
    let response = rpc(&operator, &robot_id, &ControlMessage::FetchUsageBundle).await;
    match response {
        ControlMessage::UsageBundleReport { bundle_json } => {
            let json = bundle_json.expect("bundle accumulated on the robot");
            let value: serde_json::Value =
                serde_json::from_str(&json).expect("bundle is valid JSON");
            assert_eq!(
                value["counts"]["diagnostics"]["ok"], 1,
                "one successful diagnostics invocation expected: {value}"
            );
            assert!(
                !json.contains("circuit-diag"),
                "capability name leaked into the usage bundle: {json}"
            );
        }
        other => panic!("unexpected bundle response: {other:?}"),
    }
    let response = rpc(&operator, &robot_id, &ControlMessage::FetchUsageBundle).await;
    match response {
        ControlMessage::UsageBundleReport { bundle_json } => {
            assert!(
                bundle_json.is_none(),
                "second fetch must be empty — counters reset on fetch (no double counting)"
            );
        }
        other => panic!("unexpected second bundle response: {other:?}"),
    }
}

/// A circuit dial issued before the robot holds its relay reservation fails
/// (NO_RESERVATION) and is never retried by the swarm — the CLI dispatch path
/// re-dials the circuit periodically until the robot becomes reachable. This
/// test mirrors that loop: the operator starts dialing the circuit while the
/// robot's transport does not exist yet, the robot comes up a few seconds
/// later, and the dispatch still completes.
#[tokio::test(flavor = "multi_thread")]
async fn circuit_dial_redials_until_robot_reservation_exists() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    // Construct the robot's transport (fixing its identity and libp2p id) but
    // do NOT run its event loop yet: no relay connection, no reservation.
    let robot_dir = dirs.path().join("robot");
    let agent = Arc::new(RobotAgent::new(agent_config(&robot_dir)).expect("agent builds"));
    let robot_transport = Arc::new(
        Libp2pTransportAdapter::new(Libp2pConfig {
            key_path: robot_dir.join("identity.key"),
            listen_addrs: vec![],
            relay_addrs: vec![relay_addr.clone()],
            ..Default::default()
        })
        .await
        .expect("adapter builds"),
    );
    let robot_libp2p_id = *robot_transport.libp2p_peer_id();
    let robot_gang_id = robot_transport.local_peer_id_for_test();

    // Operator connects to the relay and starts dialing the circuit NOW,
    // while the robot is not reachable.
    let operator_dir = dirs.path().join("operator");
    let operator = start_adapter(Libp2pConfig {
        key_path: operator_dir.join("identity.key"),
        listen_addrs: vec![],
        ..Default::default()
    })
    .await;
    let operator_kp = Keypair::load(&operator_dir.join("identity.key")).unwrap();
    operator
        .dial_multiaddr(&relay_addr)
        .await
        .expect("relay dial queued");
    wait_until("operator connection to the relay", async || {
        !operator.connected_peers().await.is_empty()
    })
    .await;

    let circuit = format!("{relay_addr}/p2p-circuit/p2p/{robot_libp2p_id}");
    operator
        .dial_multiaddr(&circuit)
        .await
        .expect("circuit dial queued");

    // Bring the robot up ~2s later, exactly as a slow-starting agent would.
    let late_agent = Arc::clone(&agent);
    let late_transport = Arc::clone(&robot_transport);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        late_agent
            .serve(late_transport.as_ref())
            .await
            .expect("agent serves control protocol");
        let looped = Arc::clone(&late_transport);
        tokio::spawn(async move {
            let _ = looped.run_event_loop().await;
        });
    });

    // The CLI dispatch loop: poll for the robot connection, re-dialing the
    // circuit every 2s (a failed circuit dial is never retried by the swarm).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(WAIT_SECS);
    let mut last_dial = tokio::time::Instant::now();
    loop {
        if operator
            .connected_peers()
            .await
            .iter()
            .any(|(id, _)| *id == robot_gang_id)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out: circuit re-dials never reached the late-starting robot"
        );
        if last_dial.elapsed() >= Duration::from_secs(2) {
            last_dial = tokio::time::Instant::now();
            let _ = operator.dial_multiaddr(&circuit).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // The connection is usable: a real deploy round-trip completes.
    let component = b"gang-test-component (late reservation)".to_vec();
    let response = rpc(
        &operator,
        &robot_gang_id,
        &deploy_message(&operator_kp, "late-diag", &component),
    )
    .await;
    assert!(
        matches!(
            response,
            ControlMessage::InvokeResult {
                status: InvokeStatus::Success,
                ..
            }
        ),
        "deploy after late reservation failed: {response:?}"
    );
}

/// With a non-empty trust store on the robot, a deployer whose identity is
/// not trusted is rejected even when the manifest is signed by a trusted
/// author.
#[tokio::test(flavor = "multi_thread")]
async fn circuit_dispatch_rejects_untrusted_deployer() {
    let dirs = tempfile::TempDir::new().unwrap();
    let robot_dir = dirs.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();

    // Trusted author whose key never belongs to the operator.
    let trusted_author = Keypair::generate();
    let mut trust = TrustStore::default();
    trust.add(TrustedPeer {
        peer_id: trusted_author.peer_id(),
        name: "trusted-author".into(),
        public_key: trusted_author.public_key().to_bytes().to_vec(),
    });
    trust.save(&robot_dir.join("trusted_peers.json")).unwrap();

    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;
    let (_agent, robot_transport) = start_robot(&robot_dir, Some(&relay_addr)).await;
    let (operator, _operator_kp, _via_relay) =
        connect_operator_via_circuit(&dirs.path().join("operator"), &relay_addr, &robot_transport)
            .await;

    let robot_id = robot_transport.local_peer_id_for_test();
    let component = b"gang-test-component".to_vec();

    // Manifest signed by the trusted author, pushed by the UNtrusted operator:
    // the deployer identity check (SEC-03) must reject it.
    let response = rpc(
        &operator,
        &robot_id,
        &deploy_message(&trusted_author, "evil-cap", &component),
    )
    .await;
    match response {
        ControlMessage::Error { code, message, .. } => {
            assert_eq!(code, "deploy_failed");
            assert!(
                message.contains("deployer"),
                "rejection should name the deployer check: {message}"
            );
        }
        other => panic!("untrusted deployer was not rejected: {other:?}"),
    }
}

/// Flat-network variant: the operator dials the robot's direct loopback
/// address (no relay involved). Covers the open-warehouse archetype.
#[tokio::test(flavor = "multi_thread")]
async fn direct_dispatch_deploy_and_invoke() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_agent, robot_transport) = start_robot(&dirs.path().join("robot"), None).await;

    let mut robot_addr = String::new();
    wait_until("robot to report its resolved listen address", async || {
        if let Some(a) = robot_transport
            .listen_addrs()
            .await
            .iter()
            .find(|a| a.contains("/tcp/"))
        {
            robot_addr = a.clone();
            true
        } else {
            false
        }
    })
    .await;

    let operator_dir = dirs.path().join("operator");
    let operator = start_adapter(Libp2pConfig {
        key_path: operator_dir.join("identity.key"),
        listen_addrs: vec![],
        ..Default::default()
    })
    .await;
    let operator_kp = Keypair::load(&operator_dir.join("identity.key")).unwrap();

    let dial = format!("{robot_addr}/p2p/{}", robot_transport.libp2p_peer_id());
    operator.dial_multiaddr(&dial).await.expect("dial queued");

    let robot_id = robot_transport.local_peer_id_for_test();
    let mut via_relay = true;
    wait_until(
        "operator's direct connection to the robot",
        async || match operator
            .connected_peers()
            .await
            .iter()
            .find(|(id, _)| *id == robot_id)
        {
            Some((_, relayed)) => {
                via_relay = *relayed;
                true
            }
            None => false,
        },
    )
    .await;
    assert!(!via_relay, "direct dial must not be relayed");

    let component = b"gang-test-component".to_vec();
    let response = rpc(
        &operator,
        &robot_id,
        &deploy_message(&operator_kp, "direct-diag", &component),
    )
    .await;
    assert!(
        matches!(
            response,
            ControlMessage::InvokeResult {
                status: InvokeStatus::Success,
                ..
            }
        ),
        "deploy over direct dial failed: {response:?}"
    );

    let response = rpc(&operator, &robot_id, &invoke_message("direct-diag")).await;
    match response {
        ControlMessage::InvokeResult { status, output, .. } => {
            assert!(matches!(status, InvokeStatus::Success));
            let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
            assert!(value.get("system_info").is_some());
        }
        other => panic!("unexpected invoke response: {other:?}"),
    }
}
