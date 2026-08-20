//! Integration test for the `gang up` fleet wiring.
//!
//! `gang up` is a foreground, blocking command, so this test does not shell out
//! to it — instead it assembles the exact same fleet the `up` handler builds
//! (a loopback circuit-relay, a `RobotAgent` loading a REAL default-deny policy
//! from disk plus a trust store that trusts the operator, and an operator
//! transport dialing in through the relay circuit) and drives control messages
//! over it.
//!
//! It proves the two claims that matter for the first-run experience:
//!   1. the signed sample — which declares only `ganglion:diagnostics/collect`
//!      — deploys and invokes successfully through the relay; and
//!   2. a capability declaring a group the policy does NOT permit
//!      (`ganglion:network/probe`) is rejected at deploy time, i.e. default-deny
//!      is genuinely enforced (this is not the permissive dev fallback).
//!
//! All listeners use ephemeral loopback ports; no docker, no PATH assumptions.

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

const WAIT_SECS: u64 = 30;

/// The same default-deny policy `gang up` writes: only the diagnostics group is
/// permitted, and only the named operator may deploy.
fn up_default_deny_policy(operator: &PeerId) -> String {
    format!(
        "[[capability_rules]]\n\
         group = \"ganglion:diagnostics/collect\"\n\
         allowed_patterns = [\"**\"]\n\
         \n\
         [[peer_rules]]\n\
         peer_id = \"{operator}\"\n\
         can_deploy = true\n"
    )
}

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
        key_path: dir.join("identity.key"),
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
        Arc::clone(&relay),
        format!("{tcp_addr}/p2p/{}", relay.libp2p_peer_id()),
    )
}

/// Robot agent wired exactly like `gang up`: a real default-deny policy on
/// disk, a trust store that trusts the operator, reachable via the relay only.
async fn start_up_robot(
    dir: &Path,
    operator: &Keypair,
    relay_dial_addr: &str,
) -> Arc<Libp2pTransportAdapter> {
    std::fs::create_dir_all(dir).unwrap();

    let mut trust = TrustStore::default();
    trust.add(TrustedPeer {
        peer_id: operator.peer_id(),
        name: "up-operator".into(),
        public_key: operator.public_key().to_bytes().to_vec(),
    });
    let trust_path = dir.join("trusted_peers.json");
    trust.save(&trust_path).unwrap();

    let policy_path = dir.join("policy.toml");
    std::fs::write(&policy_path, up_default_deny_policy(&operator.peer_id())).unwrap();

    let agent = Arc::new(
        RobotAgent::new(AgentConfig {
            key_path: dir.join("identity.key"),
            policy_path: Some(policy_path),
            trust_store_path: trust_path,
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
        })
        .expect("agent builds"),
    );

    let transport = start_adapter(Libp2pConfig {
        key_path: dir.join("identity.key"),
        listen_addrs: vec![],
        relay_addrs: vec![relay_dial_addr.to_string()],
        ..Default::default()
    })
    .await;

    agent.serve(transport.as_ref()).await.expect("agent serves");
    // Keep the agent alive for the duration of the test.
    Box::leak(Box::new(agent));

    wait_until("robot circuit reservation", async || {
        transport
            .listen_addrs()
            .await
            .iter()
            .any(|a| a.contains("p2p-circuit"))
    })
    .await;
    transport
}

async fn connect_operator(
    operator: &Keypair,
    key_path: &Path,
    relay_dial_addr: &str,
    robot: &Arc<Libp2pTransportAdapter>,
) -> Arc<Libp2pTransportAdapter> {
    let _ = operator; // key already persisted at key_path
    let op = start_adapter(Libp2pConfig {
        key_path: key_path.to_path_buf(),
        listen_addrs: vec![],
        ..Default::default()
    })
    .await;
    op.dial_multiaddr(relay_dial_addr)
        .await
        .expect("relay dial");
    wait_until("operator→relay connection", async || {
        !op.connected_peers().await.is_empty()
    })
    .await;

    let circuit = format!(
        "{relay_dial_addr}/p2p-circuit/p2p/{}",
        robot.libp2p_peer_id()
    );
    op.dial_multiaddr(&circuit).await.expect("circuit dial");
    let robot_id = {
        use gang_core::transport::TransportAdapter;
        robot.local_peer_id()
    };
    wait_until("operator→robot circuit connection", async || {
        op.connected_peers()
            .await
            .iter()
            .any(|(id, _)| *id == robot_id)
    })
    .await;
    op
}

fn manifest_cbor(
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
        credential_slots: vec![],
        exports: vec![],
    };
    SignedManifest::sign(&manifest, author)
        .expect("sign")
        .to_cbor()
        .expect("encode")
}

async fn rpc(op: &Libp2pTransportAdapter, robot: &PeerId, msg: &ControlMessage) -> ControlMessage {
    let bytes = encode_message(msg).expect("encode");
    let resp = op
        .send_rpc_with_timeout(robot, bytes, Duration::from_secs(WAIT_SECS))
        .await
        .expect("rpc completes");
    assert!(!resp.is_empty(), "empty response");
    decode_message::<ControlMessage>(&resp).expect("decode").0
}

/// The signed sample (diagnostics) deploys and invokes; an undeclared group
/// (network) is denied by the default-deny policy at deploy time.
#[tokio::test(flavor = "multi_thread")]
async fn up_fleet_runs_sample_and_denies_undeclared_capability() {
    let dirs = tempfile::TempDir::new().unwrap();

    // Operator identity, persisted so both the trust store and the operator
    // transport use the same key.
    let op_key_path = dirs.path().join("operator/identity.key");
    let operator = Keypair::load_or_generate(&op_key_path).unwrap();

    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;
    let robot = start_up_robot(&dirs.path().join("robot"), &operator, &relay_addr).await;
    let op = connect_operator(&operator, &op_key_path, &relay_addr, &robot).await;
    let robot_id = {
        use gang_core::transport::TransportAdapter;
        robot.local_peer_id()
    };

    // 1. Sample capability: declares only diagnostics — permitted.
    let component = b"gang-capability-diagnostics-v0.1.0-up".to_vec();
    let deploy = ControlMessage::DeployCapability {
        name: "diagnostics".into(),
        version: "0.1.0".into(),
        manifest_cbor: manifest_cbor(
            &operator,
            "diagnostics",
            CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            },
            &component,
        ),
        component_bytes: component.clone(),
        nonce: fresh_nonce(),
        timestamp_ms: unix_millis_now(),
    };
    match rpc(&op, &robot_id, &deploy).await {
        ControlMessage::InvokeResult { status, .. } => {
            assert!(
                matches!(status, InvokeStatus::Success),
                "sample deploy failed"
            );
        }
        other => panic!("unexpected deploy response: {other:?}"),
    }

    // Invoke succeeds and returns real diagnostics.
    let invoke = ControlMessage::InvokeCapability {
        name: "diagnostics".into(),
        args: vec![],
        request_id: fresh_nonce(),
        nonce: fresh_nonce(),
        export: None,
        timestamp_ms: unix_millis_now(),
    };
    match rpc(&op, &robot_id, &invoke).await {
        ControlMessage::InvokeResult { status, output, .. } => {
            assert!(matches!(status, InvokeStatus::Success), "invoke failed");
            let v: serde_json::Value = serde_json::from_slice(&output).expect("json");
            assert!(v.get("system_info").is_some(), "missing system_info: {v}");
        }
        other => panic!("unexpected invoke response: {other:?}"),
    }

    // 2. Undeclared group: network/probe has no rule — default-deny rejects it.
    let net = b"gang-network-probe-sample".to_vec();
    let deploy_net = ControlMessage::DeployCapability {
        name: "netprobe".into(),
        version: "0.1.0".into(),
        manifest_cbor: manifest_cbor(
            &operator,
            "netprobe",
            CapabilityGroup::NetworkProbe {
                version: "1.0".into(),
            },
            &net,
        ),
        component_bytes: net,
        nonce: fresh_nonce(),
        timestamp_ms: unix_millis_now(),
    };
    match rpc(&op, &robot_id, &deploy_net).await {
        ControlMessage::Error { code, message, .. } => {
            assert_eq!(code, "deploy_failed", "wrong error code: {message}");
            assert!(
                message.contains("policy"),
                "denial should cite the policy: {message}"
            );
        }
        other => panic!("undeclared-group deploy was not denied: {other:?}"),
    }
}
