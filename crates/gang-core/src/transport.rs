use std::pin::Pin;

use futures::Stream;
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

/// A bidirectional async stream with protocol metadata.
pub struct GanglionStream {
    pub protocol: ProtocolId,
    pub remote_peer: PeerId,
    pub inner: Box<dyn AsyncReadWrite + Send + Unpin>,
}

/// Combined async read+write trait object.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// Handler for incoming streams on a protocol.
pub type StreamHandler = Box<
    dyn Fn(GanglionStream) -> Pin<Box<dyn futures::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

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
    PeerConnected {
        peer_id: PeerId,
        via_relay: bool,
    },
    /// A peer disconnected.
    PeerDisconnected {
        peer_id: PeerId,
    },
    /// Received a presence announcement.
    PresenceReceived(PresenceInfo),
    /// Connection upgraded from relay to direct.
    DirectUpgrade {
        peer_id: PeerId,
    },
}

/// The core transport adapter trait. Protocol-agnostic core, opinionated defaults.
/// libp2p is the recommended default transport but not the only valid one.
#[async_trait::async_trait]
pub trait TransportAdapter: Send + Sync {
    /// Establish a connection to a remote peer.
    async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError>;

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

    /// Shut down the transport cleanly.
    async fn shutdown(&self) -> Result<(), TransportError>;
}
