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
//! - an UNAUTHORIZED operator (not in the robot's trust store) is refused;
//! - PUSH latency (ADR-024): an event emitted on the robot reaches the operator
//!   over the real relay circuit in well under the old 1.5 s poll cadence
//!   (asserted `< 500 ms`).
//!
//! The bounded broadcast-lag → `Gap` path and the ring-eviction → `Gap` path
//! are covered deterministically by unit tests in `gang_ros::events`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gang_core::capability::CapabilityGroup;
use gang_core::events::AgentEvent;
use gang_core::identity::{Keypair, PeerId};
use gang_core::manifest::{
    ComponentManifest, ResourceLimits, SignedManifest, TrustStore, TrustedPeer,
};
use gang_core::message::{fresh_nonce, unix_millis_now};
use gang_libp2p::{EventsTransport, Libp2pConfig, Libp2pTransportAdapter};
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

/// Collect events from a live feed until `idle` passes with nothing new (the
/// snapshot + retained catch-up burst arrives back-to-back). Transport-agnostic.
async fn drain<S>(stream: &mut S, idle: Duration) -> Vec<AgentEvent>
where
    S: futures::Stream<Item = AgentEvent> + Unpin,
{
    let mut out = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(idle, stream.next()).await {
        out.push(ev);
    }
    out
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
        policy_resync_interval_secs: 0,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    }
}

/// Start a robot (agent + transport) reachable only through the relay circuit.
/// Serves both push and the poll fallback.
async fn start_robot(
    dir: &Path,
    relay_dial_addr: &str,
    config: AgentConfig,
) -> (Arc<RobotAgent>, Arc<Libp2pTransportAdapter>) {
    start_robot_with(dir, relay_dial_addr, config, true).await
}

/// Like [`start_robot`] but serves ONLY the poll fallback — it does not accept
/// the `/ganglion/events/1.0` push protocol. Simulates an older/alpha-free
/// agent for exercising the operator's `auto` push→poll fallback.
async fn start_robot_poll_only(
    dir: &Path,
    relay_dial_addr: &str,
    config: AgentConfig,
) -> (Arc<RobotAgent>, Arc<Libp2pTransportAdapter>) {
    start_robot_with(dir, relay_dial_addr, config, false).await
}

async fn start_robot_with(
    dir: &Path,
    relay_dial_addr: &str,
    config: AgentConfig,
    push: bool,
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
    if push {
        agent.serve(transport.as_ref()).await.expect("agent serves");
    } else {
        agent
            .serve_poll_only(transport.as_ref())
            .await
            .expect("agent serves (poll only)");
    }

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

    // A fresh subscription yields a presence snapshot at its head (auto → push).
    let mut stream = operator
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Auto,
        )
        .await
        .expect("subscribe");
    assert_eq!(stream.active_transport(), EventsTransport::Push);
    let first = stream.next().await;
    assert!(
        matches!(first, Some(AgentEvent::PresenceSnapshot { .. })),
        "fresh subscribe must start with a presence snapshot: {first:?}"
    );
    drop(stream);

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
    // audit append over the real circuit (delivered in the catch-up burst).
    let mut stream = operator
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Auto,
        )
        .await
        .expect("resubscribe");
    let batch = drain(&mut stream, Duration::from_millis(500)).await;
    drop(stream);

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
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Auto,
        )
        .await;
    // `EventFeed` is not `Debug`, so match rather than `expect_err`. A refusal
    // must NOT auto-fall-back to poll (poll would refuse identically).
    let err = match result {
        Ok(_) => panic!("an unauthorized operator must be refused, but a feed opened"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("refused")
            || err.to_string().contains("unauthorized")
            || err.to_string().contains("refused the event subscription"),
        "refusal should be explicit: {err}"
    );
}

/// ADR-024 PUSH latency: with the subscription open and its catch-up drained,
/// an event emitted on the robot must reach the operator over the real relay
/// circuit promptly — far under the old 1.5 s poll cadence. We assert < 500 ms.
#[tokio::test(flavor = "multi_thread")]
async fn circuit_push_delivers_event_under_500ms() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    // Empty trust store → dev-permissive: any operator may subscribe.
    let robot_dir = dirs.path().join("robot");
    let (agent, robot_transport) =
        start_robot(&robot_dir, &relay_addr, base_agent_config(&robot_dir)).await;
    let (operator, _operator_kp) =
        connect_operator(&dirs.path().join("operator"), &relay_addr, &robot_transport).await;
    let robot = robot_id(&robot_transport);

    // Open the live push feed (forced) and drain the snapshot + retained catch-up.
    let mut stream = operator
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Push,
        )
        .await
        .expect("subscribe");
    assert_eq!(stream.active_transport(), EventsTransport::Push);
    let head = stream.next().await;
    assert!(
        matches!(head, Some(AgentEvent::PresenceSnapshot { .. })),
        "fresh subscribe must start with a presence snapshot: {head:?}"
    );
    let _ = drain(&mut stream, Duration::from_millis(400)).await;

    // Emit a distinctive sentinel event on the robot and time its arrival at
    // the operator. Nothing else emits in this window (heartbeats are 15 s
    // apart), but match the sentinel to be robust against a stray beat.
    const SENTINEL_UPTIME: u64 = 4_242_424;
    let t0 = Instant::now();
    agent.event_bus().publish(|seq| AgentEvent::Heartbeat {
        seq,
        ts: chrono::Utc::now(),
        uptime_secs: SENTINEL_UPTIME,
    });

    let mut latency = None;
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_secs(WAIT_SECS), stream.next()).await
    {
        if matches!(ev, AgentEvent::Heartbeat { uptime_secs, .. } if uptime_secs == SENTINEL_UPTIME)
        {
            latency = Some(t0.elapsed());
            break;
        }
    }
    drop(stream);

    let latency = latency.expect("the pushed sentinel event must arrive");
    eprintln!("measured push latency over relay circuit: {latency:?}");
    assert!(
        latency < Duration::from_millis(500),
        "push latency {latency:?} must be under 500ms (old poll was ~1.5s)"
    );
}

/// A distinctive heartbeat sentinel emitted on the robot bus; matched on the
/// operator side to confirm a specific event traversed the feed.
const SENTINEL_UPTIME: u64 = 4_242_424;

fn emit_sentinel(agent: &Arc<RobotAgent>) {
    agent.event_bus().publish(|seq| AgentEvent::Heartbeat {
        seq,
        ts: chrono::Utc::now(),
        uptime_secs: SENTINEL_UPTIME,
    });
}

/// Read the feed until the sentinel arrives or the overall budget elapses.
async fn recv_sentinel(stream: &mut gang_libp2p::EventFeed, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(ev)) => {
                if matches!(ev, AgentEvent::Heartbeat { uptime_secs, .. } if uptime_secs == SENTINEL_UPTIME)
                {
                    return true;
                }
            }
            _ => return false,
        }
    }
}

/// `poll` mode: force the request-response poll fallback (never open a stream)
/// and confirm events still flow — the retained behavior from before push.
#[tokio::test(flavor = "multi_thread")]
async fn poll_mode_delivers_events() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    let robot_dir = dirs.path().join("robot");
    let (agent, robot_transport) =
        start_robot(&robot_dir, &relay_addr, base_agent_config(&robot_dir)).await;
    let (operator, _kp) =
        connect_operator(&dirs.path().join("operator"), &relay_addr, &robot_transport).await;
    let robot = robot_id(&robot_transport);

    let mut stream = operator
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Poll,
        )
        .await
        .expect("poll subscribe");
    assert_eq!(
        stream.active_transport(),
        EventsTransport::Poll,
        "forced poll mode must report the poll transport"
    );
    // The first poll yields the presence snapshot.
    let batch = drain(&mut stream, Duration::from_millis(500)).await;
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, AgentEvent::PresenceSnapshot { .. })),
        "poll subscribe must yield a presence snapshot: {batch:?}"
    );

    // An event emitted now is picked up on the next poll tick (~1.5s).
    emit_sentinel(&agent);
    assert!(
        recv_sentinel(&mut stream, Duration::from_secs(WAIT_SECS)).await,
        "poll fallback must deliver a freshly-emitted event"
    );
}

/// `auto` against a robot that does NOT accept the push protocol (poll-only):
/// the operator must fall back to poll automatically and still deliver events.
#[tokio::test(flavor = "multi_thread")]
async fn auto_falls_back_to_poll_when_push_unavailable() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    // Poll-only robot: serves control (and the SubscribeEvents poll) but never
    // accepts /ganglion/events/1.0 — so a push open fails with NegotiationFailed.
    let robot_dir = dirs.path().join("robot");
    let (agent, robot_transport) =
        start_robot_poll_only(&robot_dir, &relay_addr, base_agent_config(&robot_dir)).await;
    let (operator, _kp) =
        connect_operator(&dirs.path().join("operator"), &relay_addr, &robot_transport).await;
    let robot = robot_id(&robot_transport);

    let mut stream = operator
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Auto,
        )
        .await
        .expect("auto subscribe must succeed via poll fallback");
    assert_eq!(
        stream.active_transport(),
        EventsTransport::Poll,
        "auto must fall back to poll when push is unavailable"
    );
    let batch = drain(&mut stream, Duration::from_millis(500)).await;
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, AgentEvent::PresenceSnapshot { .. })),
        "auto→poll must still yield a presence snapshot: {batch:?}"
    );

    emit_sentinel(&agent);
    assert!(
        recv_sentinel(&mut stream, Duration::from_secs(WAIT_SECS)).await,
        "auto→poll must deliver a freshly-emitted event"
    );
}

/// `push` mode against a poll-only robot: the stream cannot be opened, and push
/// mode must error clearly rather than silently falling back to poll.
#[tokio::test(flavor = "multi_thread")]
async fn push_mode_errors_when_stream_unavailable() {
    let dirs = tempfile::TempDir::new().unwrap();
    let (_relay, relay_addr) = start_relay(&dirs.path().join("relay")).await;

    let robot_dir = dirs.path().join("robot");
    let (_agent, robot_transport) =
        start_robot_poll_only(&robot_dir, &relay_addr, base_agent_config(&robot_dir)).await;
    let (operator, _kp) =
        connect_operator(&dirs.path().join("operator"), &relay_addr, &robot_transport).await;
    let robot = robot_id(&robot_transport);

    let result = operator
        .subscribe_events(
            &robot,
            None,
            Duration::from_secs(WAIT_SECS),
            EventsTransport::Push,
        )
        .await;
    let err = match result {
        Ok(_) => panic!("forced push against a poll-only robot must error, not fall back"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("does not support") || err.to_string().contains("event stream"),
        "push-unavailable error should be clear: {err}"
    );
}
