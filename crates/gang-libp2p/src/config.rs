use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Public bootstrap relay multiaddr. The peer ID suffix will be populated
/// after the first deployment of relay.gang.tafy.dev. Until then, this
/// constant is a placeholder and is NOT added to default config.
#[allow(dead_code)]
pub const BOOTSTRAP_RELAY: &str = "/dns4/relay.gang.tafy.dev/tcp/4001/p2p/<PEER_ID>";

/// Which transport carries the robot→operator event feed (ADR-024).
///
/// The push path is a genuine persistent substream on `/ganglion/events/1.0`
/// (via `libp2p-stream`); the poll path is a bounded request-response loop over
/// `/ganglion/control/1.0` (`ControlMessage::SubscribeEvents`). Because
/// `libp2p-stream` is a pre-release, the poll path is retained as a fallback
/// rather than being removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventsTransport {
    /// Prefer push; fall back to poll automatically when push is unavailable
    /// (older agent, alpha misbehaving, protocol-not-supported) or if a push
    /// stream drops mid-session. The default.
    #[default]
    Auto,
    /// Force the push substream; error clearly if it cannot be opened (never
    /// silently falls back to poll).
    Push,
    /// Force the request-response poll; never open a push stream.
    Poll,
}

impl std::str::FromStr for EventsTransport {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "push" => Ok(Self::Push),
            "poll" => Ok(Self::Poll),
            other => Err(format!(
                "invalid events transport '{other}': expected auto, push, or poll"
            )),
        }
    }
}

impl std::fmt::Display for EventsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Auto => "auto",
            Self::Push => "push",
            Self::Poll => "poll",
        };
        f.write_str(s)
    }
}

/// Configuration for the libp2p transport adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Libp2pConfig {
    /// Path to the Ed25519 identity key file.
    pub key_path: PathBuf,

    /// Addresses to listen on.
    /// Default: ["/ip4/0.0.0.0/tcp/0", "/ip4/0.0.0.0/udp/0/quic-v1"]
    #[serde(default = "default_listen_addrs")]
    pub listen_addrs: Vec<String>,

    /// Relay addresses to connect to on startup.
    /// Robot agents should configure at least one relay.
    #[serde(default)]
    pub relay_addrs: Vec<String>,

    /// Bootstrap peers for Kademlia DHT.
    #[serde(default)]
    pub bootstrap_peers: Vec<String>,

    /// Whether this node acts as a relay server.
    #[serde(default)]
    pub relay_server: bool,

    /// Idle connection timeout in seconds.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,

    /// Maximum number of inbound connections.
    #[serde(default = "default_max_connections")]
    pub max_inbound_connections: u32,

    /// Enable WebTransport listener.
    ///
    /// NOTE: libp2p 0.56 only provides `webtransport-websys`, which targets
    /// browser/WASM environments. Native (server-side) WebTransport is not yet
    /// available in the libp2p Rust stack. This flag is reserved for future use
    /// when a native WebTransport transport ships. Setting it to `true` today
    /// has no effect on the swarm but will be reported in capabilities.
    #[serde(default)]
    pub enable_webtransport: bool,

    /// Enable WebRTC listener for browser-based operator UIs.
    ///
    /// NOTE: libp2p 0.56 does not ship a WebRTC transport crate. This flag is
    /// reserved for future use. Setting it to `true` today has no effect on
    /// the swarm but will be reported in capabilities.
    #[serde(default)]
    pub enable_webrtc: bool,

    /// Default transport for the robot→operator event feed (ADR-024). The
    /// operator's `auto`/`push`/`poll` selection; a per-command CLI flag
    /// overrides it. Defaults to [`EventsTransport::Auto`].
    #[serde(default)]
    pub events_transport: EventsTransport,

    /// Poll interval (milliseconds) for the event-feed poll fallback: how often
    /// the operator re-requests `SubscribeEvents` when push is unavailable or
    /// `events_transport = poll`. Ignored on the push path. Default 1500 ms.
    #[serde(default = "default_events_poll_interval_ms")]
    pub events_poll_interval_ms: u64,
}

fn default_events_poll_interval_ms() -> u64 {
    1500
}

fn default_listen_addrs() -> Vec<String> {
    vec![
        "/ip4/0.0.0.0/tcp/0".into(),
        "/ip4/0.0.0.0/udp/0/quic-v1".into(),
    ]
}

fn default_idle_timeout() -> u64 {
    300
}

fn default_max_connections() -> u32 {
    64
}

impl Default for Libp2pConfig {
    fn default() -> Self {
        Self {
            key_path: gang_core::identity::default_key_path(),
            listen_addrs: default_listen_addrs(),
            relay_addrs: Vec::new(),
            bootstrap_peers: Vec::new(),
            relay_server: false,
            idle_timeout_secs: default_idle_timeout(),
            max_inbound_connections: default_max_connections(),
            enable_webtransport: false,
            enable_webrtc: false,
            events_transport: EventsTransport::default(),
            events_poll_interval_ms: default_events_poll_interval_ms(),
        }
    }
}
