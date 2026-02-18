use std::path::PathBuf;
use serde::{Deserialize, Serialize};

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
        }
    }
}
