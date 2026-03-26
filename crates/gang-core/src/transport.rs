use std::pin::Pin;
use std::time::{Duration, Instant};

use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::TransportError;
use crate::identity::PeerId;
use crate::protocol::ProtocolId;

/// What a transport supports. gang-core selects strategies based on
/// these capabilities without knowing transport internals.
#[derive(Debug, Clone, Default)]
pub struct TransportCapabilities {
    /// Supports circuit relay (connect through a third party).
    pub relay: bool,
    /// Supports hole-punching (DCUtR or similar).
    pub hole_punch: bool,
    /// Supports direct dialing (no relay needed).
    pub direct_dial: bool,
    /// The transport provides its own encryption.
    pub encrypted: bool,
    /// Names of available concrete transports (e.g., "tcp", "quic").
    pub transports: Vec<String>,
}

/// Transport preference for happy-eyeballs selection.
/// v0.2: attempt transports in parallel, first successful handshake wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportPreference {
    /// Ordered list of preferred transports (e.g., ["quic", "tcp"]).
    /// All are attempted in parallel; this order is used for tie-breaking.
    pub preferred_order: Vec<String>,
    /// How long to wait for a connection before giving up.
    pub dial_timeout: Duration,
    /// Stagger delay between parallel attempts (happy-eyeballs style).
    /// The first transport starts immediately; subsequent ones start after
    /// this delay if the first hasn't connected yet.
    pub stagger_delay: Duration,
}

impl Default for TransportPreference {
    fn default() -> Self {
        Self {
            preferred_order: vec!["quic".into(), "tcp".into()],
            dial_timeout: Duration::from_secs(30),
            stagger_delay: Duration::from_millis(250),
        }
    }
}

/// Result of a parallel dial attempt — which transport won.
#[derive(Debug)]
pub struct DialResult {
    /// Which transport was used for the successful connection.
    pub transport_used: String,
    /// Whether the connection goes through a relay.
    pub via_relay: bool,
    /// How long the dial took.
    pub dial_duration: Duration,
    /// The established stream.
    pub stream: GanglionStream,
}

/// Per-transport statistics for a connection to a peer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransportStats {
    /// Name of the transport (e.g., "quic", "tcp", "relay").
    pub transport: String,
    /// Whether this is a relayed connection.
    pub via_relay: bool,
    /// Connection establishment time.
    pub connect_time_ms: u64,
    /// Number of messages sent.
    pub messages_sent: u64,
    /// Number of messages received.
    pub messages_received: u64,
    /// Bytes sent.
    pub bytes_sent: u64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Latest RTT measurement (from ping).
    pub last_rtt_ms: Option<u64>,
    /// Whether DCUtR upgrade was attempted.
    pub dcutr_attempted: bool,
    /// Whether DCUtR upgrade succeeded.
    pub dcutr_succeeded: bool,
    /// Connection uptime in seconds.
    pub uptime_secs: u64,
    /// Number of reconnections since initial connect.
    pub reconnections: u64,
}

/// A bidirectional async stream with protocol metadata.
pub struct GanglionStream {
    pub protocol: ProtocolId,
    pub remote_peer: PeerId,
    pub inner: Box<dyn AsyncReadWrite + Send + Unpin>,
}

impl std::fmt::Debug for GanglionStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GanglionStream")
            .field("protocol", &self.protocol)
            .field("remote_peer", &self.remote_peer)
            .finish_non_exhaustive()
    }
}

/// Combined async read+write trait object.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// Handler for incoming streams on a protocol.
pub type StreamHandler =
    Box<dyn Fn(GanglionStream) -> Pin<Box<dyn futures::Future<Output = ()> + Send>> + Send + Sync>;

/// Presence information announced by a robot agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresenceInfo {
    pub peer_id: PeerId,
    pub role: crate::identity::Role,
    pub capabilities_installed: Vec<String>,
    pub uptime_secs: u64,
    pub ganglion_version: String,
}

/// Events emitted by the transport layer to the application.
#[derive(Debug)]
pub enum TransportEvent {
    /// A new peer connected.
    PeerConnected { peer_id: PeerId, via_relay: bool },
    /// A peer disconnected.
    PeerDisconnected { peer_id: PeerId },
    /// Received a presence announcement.
    PresenceReceived(PresenceInfo),
    /// Connection upgraded from relay to direct.
    DirectUpgrade { peer_id: PeerId },
}

/// The core transport adapter trait. Protocol-agnostic core, opinionated defaults.
/// libp2p is the recommended default transport but not the only valid one.
#[async_trait::async_trait]
pub trait TransportAdapter: Send + Sync {
    /// Establish a connection to a remote peer.
    async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError>;

    /// Attempt to connect using multiple transports in parallel (happy-eyeballs).
    /// First successful handshake wins; remaining attempts are cancelled.
    /// Added in v0.2. Default implementation falls back to single `dial`.
    async fn dial_parallel(
        &self,
        peer: &PeerId,
        _preference: &TransportPreference,
    ) -> Result<DialResult, TransportError> {
        let start = Instant::now();
        let stream = self.dial(peer).await?;
        Ok(DialResult {
            transport_used: "default".into(),
            via_relay: false,
            dial_duration: start.elapsed(),
            stream,
        })
    }

    /// Register a handler for incoming streams on a specific protocol.
    async fn listen(
        &self,
        protocol: ProtocolId,
        handler: StreamHandler,
    ) -> Result<(), TransportError>;

    /// Return this node's peer ID.
    fn local_peer_id(&self) -> PeerId;

    /// Describe what this transport supports.
    fn capabilities(&self) -> TransportCapabilities;

    /// Subscribe to transport-level events.
    fn events(&self) -> Pin<Box<dyn Stream<Item = TransportEvent> + Send>>;

    /// Announce presence to authorized peers.
    async fn announce_presence(&self, info: PresenceInfo) -> Result<(), TransportError>;

    /// Get per-transport statistics for a connected peer.
    /// Added in v0.2.
    async fn transport_stats(&self, _peer: &PeerId) -> Option<TransportStats> {
        None
    }

    /// Shut down the transport cleanly.
    async fn shutdown(&self) -> Result<(), TransportError>;
}
