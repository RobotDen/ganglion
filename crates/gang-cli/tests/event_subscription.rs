//! In-process integration tests for the robot→operator event feed
//! (`/ganglion/events/1.0`).
//!
//! Each test assembles the three real roles on loopback — a circuit-relay v2
//! server, a `RobotAgent` serving control + events, and an operator transport —
//! and drives the subscription over a REAL relay circuit (the robot binds no
//! direct listener, so the relay path is the only way in). Everything uses
//! ephemeral loopback ports; no Docker.
//!
//! Coverage:
//! - authorized subscribe → `PresenceSnapshot`;
//! - a policy DENY (deploying an undeclared-capability component) puts a
//!   `PolicyDecision{Deny}` on the feed;
//! - a legit deploy+invoke puts `AuditAppended` + `PolicyDecision{Allow}` on
//!   the feed;
//! - an UNAUTHORIZED operator (not in the robot's trust store) is refused.
//!
//! The bounded broadcast-lag → `Gap` path and the ring-eviction → `Gap` path
//! are covered deterministically by unit tests in `gang_ros::events`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gang_core::capability::CapabilityGroup;
use gang_core::events::AgentEvent;
use gang_core::identity::{Keypair, PeerId};
use gang_core::manifest::{
    ComponentManifest, ResourceLimits, SignedManifest, TrustStore, TrustedPeer,
};
use gang_core::message::{fresh_nonce, unix_millis_now};
use gang_libp2p::{Libp2pConfig, Libp2pTransportAdapter};
use gang_ros::agent::{AgentConfig, RobotAgent};
use gang_ros::filesystem::FsRule;

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

    let mut tcp_addr = String::new();
    wait_until("relay listen address", async || {
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
    (
        relay.clone(),
        format!("{tcp_addr}/p2p/{}", relay.libp2p_peer_id()),
    )
}

fn base_agent_config(dir: &Path) -> AgentConfig {
    AgentConfig {
        key_path: dir.join("identity.key"),
        policy_path: None,
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

/// Start a robot (agent + transport) reachable only through the relay circuit.
async fn start_robot(
    dir: &Path,
    relay_dial_addr: &str,
    config: AgentConfig,
) -> (Arc<RobotAgent>, Arc<Libp2pTransportAdapter>) {
    std::fs::create_dir_all(dir).unwrap();
    let agent = Arc::new(RobotAgent::new(config).expect("agent builds"));
    let transport = start_adapter(Libp2pConfig {
        key_path: dir.join("identity.key"),
        listen_addrs: vec![],
        relay_addrs: vec![relay_dial_addr.to_string()],
        ..Default::default()
    })
    .await;
    agent.serve(transport.as_ref()).await.expect("agent serves");

    wait_until("robot's relay circuit reservation", async || {
        transport
            .listen_addrs()
            .await
            .iter()
            .any(|a| a.contains("p2p-circuit"))
    })
    .await;
    (agent, transport)
}

async fn connect_operator(
    dir: &Path,
    relay_dial_addr: &str,
    robot: &Arc<Libp2pTransportAdapter>,
) -> (Arc<Libp2pTransportAdapter>, Keypair) {
    std::fs::create_dir_all(dir).unwrap();
    let key_path = dir.join("identity.key");
    let operator = start_adapter(Libp2pConfig {
        key_path: key_path.clone(),
        listen_addrs: vec![],
        ..Default::default()
    })
    .await;
    let keypair = Keypair::load(&key_path).expect("operator key");

    operator
        .dial_multiaddr(relay_dial_addr)
        .await
        .expect("relay dial queued");
    wait_until("operator→relay connection", async || {
        !operator.connected_peers().await.is_empty()
    })
    .await;

    let robot_gang = {
        use gang_core::transport::TransportAdapter;
        robot.local_peer_id()
    };
    let circuit = format!(
        "{relay_dial_addr}/p2p-circuit/p2p/{}",
        robot.libp2p_peer_id()
    );
    operator
        .dial_multiaddr(&circuit)
        .await
        .expect("circuit dial queued");
    wait_until("operator→robot circuit connection", async || {
        operator
            .connected_peers()
            .await
            .iter()
            .any(|(id, _)| *id == robot_gang)
    })
    .await;
    (operator, keypair)
}

fn robot_id(robot: &Libp2pTransportAdapter) -> PeerId {
    use gang_core::transport::TransportAdapter;
    robot.local_peer_id()
}

fn signed_manifest(
    author: &Keypair,
    name: &str,
    group: CapabilityGroup,
    component: &[u8],
) -> Vec<u8> {
    let manifest = ComponentManifest {
        schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
        name: name.into(),
        version: "0.1.0".into(),
        declared_capabilities: vec![group],
        author_peer_id: author.peer_id(),
        component_hash: blake3::hash(component).to_hex().to_string(),
        limits: ResourceLimits::default(),
        language: Default::default(),
        description: String::new(),
        tags: vec![],
        min_ganglion_version: None,
    };
    SignedManifest::sign(&manifest, author)
        .expect("sign")
        .to_cbor()
        .expect("cbor")
}

async fn deploy(
    operator: &Libp2pTransportAdapter,
    robot: &PeerId,
    author: &Keypair,
    name: &str,
    group: CapabilityGroup,
) -> gang_core::message::ControlMessage {
    use gang_core::message::{ControlMessage, decode_message, encode_message};
    let component = format!("component-bytes-{name}").into_bytes();
    let msg = ControlMessage::DeployCapability {
        name: name.into(),
        version: "0.1.0".into(),
        manifest_cbor: signed_manifest(author, name, group, &component),
        component_bytes: component,
        nonce: fresh_nonce(),
        timestamp_ms: unix_millis_now(),
    };
    let bytes = encode_message(&msg).unwrap();
    let resp = operator
        .send_rpc_with_timeout(robot, bytes, Duration::from_secs(WAIT_SECS))
        .await
        .expect("deploy rpc");
    decode_message::<ControlMessage>(&resp).unwrap().0
}

async fn invoke(operator: &Libp2pTransportAdapter, robot: &PeerId, name: &str) {
    use gang_core::message::{ControlMessage, encode_message};
    let msg = ControlMessage::InvokeCapability {
        name: name.into(),
        args: vec![],
        request_id: fresh_nonce(),
        nonce: fresh_nonce(),
        timestamp_ms: unix_millis_now(),
    };
    let bytes = encode_message(&msg).unwrap();
    let _ = operator
        .send_rpc_with_timeout(robot, bytes, Duration::from_secs(WAIT_SECS))
        .await
        .expect("invoke rpc");
}

/// Fresh subscribe over the circuit: snapshot, then deny path, then allow+audit.
#[tokio::test(flavor = "multi_thread")]
async fn circuit_subscription_snapshot_deny_and_audit() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    // Restrictive policy: only diagnostics permitted; deploy is open.
    let robot_dir = dirs.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();
    let policy_toml = r#"
[[capability_rules]]
group = "ganglion:diagnostics/collect"
allowed_patterns = ["**"]

[[peer_rules]]
peer_id = "*"
can_deploy = true
"#;
    let policy_path = robot_dir.join("policy.toml");
    std::fs::write(&policy_path, policy_toml).unwrap();
    let mut config = base_agent_config(&robot_dir);
    config.policy_path = Some(policy_path);

    let (_agent, robot_transport) = start_robot(&robot_dir, &relay_addr, config).await;
    let (operator, operator_kp) =
        connect_operator(&dirs.path().join("operator"), &relay_addr, &robot_transport).await;
    let robot = robot_id(&robot_transport);

    // A fresh subscription yields a presence snapshot at its head.
    let batch = operator
        .subscribe_events(&robot, None, Duration::from_secs(WAIT_SECS))
        .await
        .expect("subscribe");
    assert!(
        matches!(batch.first(), Some(AgentEvent::PresenceSnapshot { .. })),
        "fresh subscribe must start with a presence snapshot: {batch:?}"
    );

    // Deny path: deploy a process/spawn capability the policy forbids.
    let denied = deploy(
        &operator,
        &robot,
        &operator_kp,
        "needs-spawn",
        CapabilityGroup::ProcessSpawn {
            version: "1.0".into(),
            allowed_commands: vec!["ls".into()],
        },
    )
    .await;
    assert!(
        matches!(denied, gang_core::message::ControlMessage::Error { .. }),
        "process/spawn deploy must be denied: {denied:?}"
    );

    // Allow path: deploy + invoke diagnostics (broker path).
    let ok = deploy(
        &operator,
        &robot,
        &operator_kp,
        "diag",
        CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        },
    )
    .await;
    assert!(
        matches!(ok, gang_core::message::ControlMessage::InvokeResult { .. }),
        "diagnostics deploy must succeed: {ok:?}"
    );
    invoke(&operator, &robot, "diag").await;

    // Re-subscribe (fresh): the feed now carries the deny, the allow, and the
    // audit append over the real circuit.
    let batch = operator
        .subscribe_events(&robot, None, Duration::from_secs(WAIT_SECS))
        .await
        .expect("resubscribe");

    let has = |pred: fn(&AgentEvent) -> bool| batch.iter().any(pred);
    assert!(
        has(|e| matches!(
            e,
            AgentEvent::PolicyDecision {
                decision: gang_core::events::PolicyOutcome::Deny,
                ..
            }
        )),
        "expected a PolicyDecision Deny on the feed: {batch:?}"
    );
    assert!(
        has(|e| matches!(
            e,
            AgentEvent::PolicyDecision {
                decision: gang_core::events::PolicyOutcome::Allow,
                ..
            }
        )),
        "expected a PolicyDecision Allow on the feed: {batch:?}"
    );
    assert!(
        has(|e| matches!(e, AgentEvent::AuditAppended { .. })),
        "expected an AuditAppended on the feed: {batch:?}"
    );
}

/// An operator not in the robot's trust store cannot subscribe: the robot
/// refuses and streams nothing (no presence snapshot).
#[tokio::test(flavor = "multi_thread")]
async fn circuit_subscription_rejects_unauthorized_operator() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    // Robot trusts only some other author — NOT the operator that will connect.
    let robot_dir = dirs.path().join("robot");
    std::fs::create_dir_all(&robot_dir).unwrap();
    let trusted_other = Keypair::generate();
    let mut trust = TrustStore::default();
    trust.add(TrustedPeer {
        peer_id: trusted_other.peer_id(),
        name: "someone-else".into(),
        public_key: trusted_other.public_key().to_bytes().to_vec(),
    });
    trust.save(&robot_dir.join("trusted_peers.json")).unwrap();

    let (_agent, robot_transport) =
        start_robot(&robot_dir, &relay_addr, base_agent_config(&robot_dir)).await;
    let (operator, _operator_kp) =
        connect_operator(&dirs.path().join("operator"), &relay_addr, &robot_transport).await;
    let robot = robot_id(&robot_transport);

    let result = operator
        .subscribe_events(&robot, None, Duration::from_secs(WAIT_SECS))
        .await;
    let err = result.expect_err("an unauthorized operator must be refused");
    assert!(
        err.to_string().contains("refused") || err.to_string().contains("unauthorized"),
        "refusal should be explicit: {err}"
    );
}
