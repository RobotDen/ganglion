use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Public bootstrap relay multiaddr. The peer ID suffix will be populated
/// after the first deployment of relay.gang.tafy.dev. Until then, this
/// constant is a placeholder and is NOT added to default config.
#[allow(dead_code)]
pub const BOOTSTRAP_RELAY: &str = "/dns4/relay.gang.tafy.dev/tcp/4001/p2p/<PEER_ID>";

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
    /// NOTE: libp2p 0.54 only provides `webtransport-websys`, which targets
    /// browser/WASM environments. Native (server-side) WebTransport is not yet
    /// available in the libp2p Rust stack. This flag is reserved for future use
    /// when a native WebTransport transport ships. Setting it to `true` today
    /// has no effect on the swarm but will be reported in capabilities.
    #[serde(default)]
    pub enable_webtransport: bool,

    /// Enable WebRTC listener for browser-based operator UIs.
    ///
    /// NOTE: libp2p 0.54 does not ship a WebRTC transport crate. This flag is
    /// reserved for future use. Setting it to `true` today has no effect on
    /// the swarm but will be reported in capabilities.
    #[serde(default)]
    pub enable_webrtc: bool,
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
        }
    }
}
