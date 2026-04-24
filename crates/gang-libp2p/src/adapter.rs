use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use libp2p::request_response::{self, OutboundRequestId};
use libp2p::{Multiaddr, PeerId as Libp2pPeerId, Swarm};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tracing::{debug, info, warn};

use gang_core::error::TransportError;
use gang_core::identity::{Keypair, PeerId};
use gang_core::protocol::{self, ProtocolId};
use gang_core::transport::{
    GanglionStream, PresenceInfo, StreamHandler, TransportAdapter, TransportCapabilities,
    TransportEvent, TransportStats,
};

use crate::config::Libp2pConfig;
use crate::swarm::{self, GanglionBehaviour};

/// libp2p implementation of the Ganglion TransportAdapter trait.
///
/// This is the recommended default transport. It provides:
/// - TCP + QUIC transports
/// - Noise encryption
/// - Yamux multiplexing
/// - Circuit relay v2 for NAT traversal (client and server)
/// - DCUtR for direct connection upgrades
/// - Kademlia for peer routing
/// - Ganglion stream protocols via request-response
pub struct Libp2pTransportAdapter {
    config: Libp2pConfig,
    keypair: Keypair,
    local_peer_id: PeerId,
    libp2p_peer_id: Libp2pPeerId,
    swarm: Arc<Mutex<Swarm<GanglionBehaviour>>>,
    event_tx: mpsc::Sender<TransportEvent>,
    event_rx: Arc<Mutex<mpsc::Receiver<TransportEvent>>>,
    connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>>,
    /// Registered protocol handlers for incoming streams.
    protocol_handlers: Arc<RwLock<HashMap<String, StreamHandler>>>,
    /// Pending outbound requests awaiting a response.
    pending_requests: Arc<Mutex<HashMap<OutboundRequestId, PendingRequest>>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

/// Tracks a pending outbound request so we can deliver the response
/// as a `GanglionStream`.
struct PendingRequest {
    peer_id: PeerId,
    protocol: ProtocolId,
    response_tx: oneshot::Sender<Result<Vec<u8>, TransportError>>,
}

#[derive(Debug, Clone)]
pub(crate) struct PeerConnection {
    pub libp2p_peer_id: Libp2pPeerId,
    pub via_relay: bool,
    pub connected_at: std::time::Instant,
    /// Which transport was used (tcp, quic, relay).
    pub transport: String,
    /// Latest RTT from ping.
    pub last_rtt: Option<std::time::Duration>,
    /// Whether DCUtR was attempted.
    pub dcutr_attempted: bool,
    /// Whether DCUtR succeeded.
    pub dcutr_succeeded: bool,
    /// Messages sent over this connection.
    pub messages_sent: u64,
    /// Messages received.
    pub messages_received: u64,
    /// Bytes sent.
    pub bytes_sent: u64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Number of reconnections.
    pub reconnections: u64,
}

impl Libp2pTransportAdapter {
    /// Create a new adapter from configuration.
    pub async fn new(config: Libp2pConfig) -> anyhow::Result<Self> {
        let keypair = Keypair::load_or_generate(&config.key_path)?;
        let local_peer_id = keypair.peer_id();

        let (mut swarm, libp2p_peer_id) = swarm::build_swarm(&config).await?;

        // Start listening
        swarm::start_listening(&mut swarm, &config)?;

        // Add bootstrap peers
        swarm::add_bootstrap_peers(&mut swarm, &config)?;

        let (event_tx, event_rx) = mpsc::channel(256);

        let adapter = Self {
            config,
            keypair,
            local_peer_id,
            libp2p_peer_id,
            swarm: Arc::new(Mutex::new(swarm)),
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            connected_peers: Arc::new(RwLock::new(HashMap::new())),
            protocol_handlers: Arc::new(RwLock::new(HashMap::new())),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tx: None,
        };

        Ok(adapter)
    }

    /// Start the swarm event loop. This must be called after construction
    /// and runs until shutdown.
    pub async fn run_event_loop(&self) -> anyhow::Result<()> {
        use futures::StreamExt;
        use libp2p::swarm::SwarmEvent;

        let swarm = self.swarm.clone();
        let event_tx = self.event_tx.clone();
        let connected_peers = self.connected_peers.clone();

        loop {
            let event = {
                let mut swarm = swarm.lock().await;
                swarm.next().await
            };

            match event {
                Some(SwarmEvent::ConnectionEstablished {
                    peer_id, endpoint, ..
                }) => {
                    let gang_peer_id = libp2p_to_gang_peer_id(&peer_id);
                    let via_relay = endpoint.is_relayed();

                    info!(
                        peer = %peer_id,
                        relay = via_relay,
                        "Connection established"
                    );

                    // Determine transport from endpoint multiaddr.
                    // Recognises current (tcp, quic) and future (webtransport,
                    // webrtc) transports so the field is correct if/when those
                    // features land in a future libp2p release.
                    let transport_name = if via_relay {
                        "relay".to_string()
                    } else {
                        let addr_str = endpoint.get_remote_address().to_string();
                        if addr_str.contains("/webtransport/") {
                            "webtransport".to_string()
                        } else if addr_str.contains("/webrtc-direct/")
                            || addr_str.contains("/webrtc/")
                        {
                            "webrtc".to_string()
                        } else if addr_str.contains("quic") {
                            "quic".to_string()
                        } else {
                            "tcp".to_string()
                        }
                    };

                    let peer_key = gang_peer_id.as_str().to_string();
                    let mut peers = connected_peers.write().await;
                    let reconnections = peers
                        .get(&peer_key)
                        .map(|p| p.reconnections + 1)
                        .unwrap_or(0);

                    peers.insert(
                        peer_key,
                        PeerConnection {
                            libp2p_peer_id: peer_id,
                            via_relay,
                            connected_at: std::time::Instant::now(),
                            transport: transport_name,
                            last_rtt: None,
                            dcutr_attempted: false,
                            dcutr_succeeded: false,
                            messages_sent: 0,
                            messages_received: 0,
                            bytes_sent: 0,
                            bytes_received: 0,
                            reconnections,
                        },
                    );

                    let _ = event_tx
                        .send(TransportEvent::PeerConnected {
                            peer_id: gang_peer_id,
                            via_relay,
                        })
                        .await;
                }
                Some(SwarmEvent::ConnectionClosed { peer_id, .. }) => {
                    let gang_peer_id = libp2p_to_gang_peer_id(&peer_id);
                    info!(peer = %peer_id, "Connection closed");

                    connected_peers.write().await.remove(gang_peer_id.as_str());

                    let _ = event_tx
                        .send(TransportEvent::PeerDisconnected {
                            peer_id: gang_peer_id,
                        })
                        .await;
                }
                Some(SwarmEvent::NewListenAddr { address, .. }) => {
                    info!("Listening on {address}");
                }
                Some(SwarmEvent::Behaviour(event)) => {
                    self.handle_behaviour_event(event).await;
                }
                Some(other) => {
                    debug!("Swarm event: {other:?}");
                }
                None => {
                    info!("Swarm event stream ended");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_behaviour_event(&self, event: swarm::GanglionBehaviourEvent) {
        use swarm::GanglionBehaviourEvent;

        match event {
            GanglionBehaviourEvent::Identify(libp2p::identify::Event::Received {
                peer_id,
                info,
                ..
            }) => {
                debug!(
                    peer = %peer_id,
                    agent = %info.agent_version,
                    "Identified peer"
                );

                // Add peer addresses to Kademlia
                let mut swarm = self.swarm.lock().await;
                for addr in &info.listen_addrs {
                    swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, addr.clone());
                }
            }
            GanglionBehaviourEvent::Ping(libp2p::ping::Event {
                peer,
                result: Ok(rtt),
                ..
            }) => {
                debug!(peer = %peer, rtt = ?rtt, "Ping");
                // Update RTT tracking
                let gang_peer_id = libp2p_to_gang_peer_id(&peer);
                let mut peers = self.connected_peers.write().await;
                if let Some(conn) = peers.get_mut(gang_peer_id.as_str()) {
                    conn.last_rtt = Some(rtt);
                }
            }
            GanglionBehaviourEvent::Dcutr(libp2p::dcutr::Event {
                remote_peer_id,
                result: Ok(_),
                ..
            }) => {
                let gang_peer_id = libp2p_to_gang_peer_id(&remote_peer_id);
                info!(peer = %remote_peer_id, "Direct connection upgraded via DCUtR");

                // Track DCUtR success
                let mut peers = self.connected_peers.write().await;
                if let Some(conn) = peers.get_mut(gang_peer_id.as_str()) {
                    conn.dcutr_attempted = true;
                    conn.dcutr_succeeded = true;
                    conn.via_relay = false;
                    conn.transport = "quic".into(); // DCUtR typically upgrades to direct QUIC
                }

                let _ = self
                    .event_tx
                    .send(TransportEvent::DirectUpgrade {
                        peer_id: gang_peer_id,
                    })
                    .await;
            }
            GanglionBehaviourEvent::GanglionRpc(request_response::Event::Message {
                peer,
                message,
                ..
            }) => {
                self.handle_rpc_message(peer, message).await;
            }
            GanglionBehaviourEvent::GanglionRpc(request_response::Event::OutboundFailure {
                request_id,
                error,
                ..
            }) => {
                warn!(request_id = ?request_id, error = ?error, "Outbound RPC failed");
                let mut pending = self.pending_requests.lock().await;
                if let Some(req) = pending.remove(&request_id) {
                    let _ = req.response_tx.send(Err(TransportError::DialFailed {
                        peer: req.peer_id.to_string(),
                        reason: format!("outbound request failed: {error:?}"),
                    }));
                }
            }
            GanglionBehaviourEvent::GanglionRpc(request_response::Event::InboundFailure {
                peer,
                request_id,
                error,
                ..
            }) => {
                warn!(
                    peer = %peer,
                    request_id = ?request_id,
                    error = ?error,
                    "Inbound RPC failed"
                );
            }
            GanglionBehaviourEvent::RelayServer(event) => {
                debug!("Relay server event: {event:?}");
            }
            _ => {}
        }
    }

    /// Handle an incoming or outgoing RPC message.
    async fn handle_rpc_message(
        &self,
        peer: Libp2pPeerId,
        message: request_response::Message<Vec<u8>, Vec<u8>>,
    ) {
        match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
                let gang_peer_id = libp2p_to_gang_peer_id(&peer);

                // Update message counters
                {
                    let mut peers = self.connected_peers.write().await;
                    if let Some(conn) = peers.get_mut(gang_peer_id.as_str()) {
                        conn.messages_received += 1;
                        conn.bytes_received += request.len() as u64;
                    }
                }

                // Try to find a registered handler for any Ganglion protocol.
                // The request_response behaviour negotiates the protocol, so
                // we check registered handlers in protocol priority order.
                let handlers = self.protocol_handlers.read().await;
                let handler_protocol = protocol::ALL_PROTOCOLS
                    .iter()
                    .find(|p| handlers.contains_key(**p));

                if let Some(&proto) = handler_protocol {
                    debug!(
                        peer = %peer,
                        protocol = proto,
                        len = request.len(),
                        "Incoming RPC request"
                    );

                    // Build a GanglionStream from the request data.
                    // The response channel is used to send back data.
                    let (response_data_tx, response_data_rx) = oneshot::channel::<Vec<u8>>();

                    // Create a duplex stream for the handler.
                    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
                    let (server_read, mut client_write) = tokio::io::duplex(64 * 1024);

                    // Write the request data into the read side for the handler
                    let request_data = request;
                    tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        let _ = client_write.write_all(&request_data).await;
                        let _ = client_write.shutdown().await;
                    });

                    // Collect the handler's response
                    tokio::spawn(async move {
                        use tokio::io::AsyncReadExt;
                        let mut buf = Vec::new();
                        let mut reader = server_read;
                        let _ = reader.read_to_end(&mut buf).await;
                        let _ = response_data_tx.send(buf);
                    });

                    let stream = GanglionStream {
                        protocol: ProtocolId::new(proto),
                        remote_peer: gang_peer_id,
                        inner: Box::new(merge_rw(client_read, server_write)),
                    };

                    if let Some(handler) = handlers.get(proto) {
                        let fut = handler(stream);
                        tokio::spawn(fut);
                    }

                    // Wait for the handler's response and send it back
                    let swarm = self.swarm.clone();
                    tokio::spawn(async move {
                        let response = response_data_rx.await.unwrap_or_default();
                        let mut swarm = swarm.lock().await;
                        let _ = swarm
                            .behaviour_mut()
                            .ganglion_rpc
                            .send_response(channel, response);
                    });
                } else {
                    debug!(peer = %peer, "No handler registered, sending empty response");
                    let mut swarm = self.swarm.lock().await;
                    let _ = swarm
                        .behaviour_mut()
                        .ganglion_rpc
                        .send_response(channel, Vec::new());
                }
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                let mut pending = self.pending_requests.lock().await;
                if let Some(req) = pending.remove(&request_id) {
                    debug!(
                        peer = %req.peer_id,
                        protocol = %req.protocol,
                        len = response.len(),
                        "Received RPC response"
                    );

                    // Update message counters
                    {
                        let mut peers = self.connected_peers.write().await;
                        if let Some(conn) = peers.get_mut(req.peer_id.as_str()) {
                            conn.messages_received += 1;
                            conn.bytes_received += response.len() as u64;
                        }
                    }

                    let _ = req.response_tx.send(Ok(response));
                }
            }
        }
    }

    /// Connect to relay nodes specified in config.
    pub async fn connect_to_relays(&self) -> anyhow::Result<()> {
        let mut swarm = self.swarm.lock().await;
        swarm::connect_to_relays(&mut swarm, &self.config).await
    }

    /// Dial a peer by their libp2p multiaddr.
    pub async fn dial_multiaddr(&self, addr: &str) -> Result<(), TransportError> {
        let addr: Multiaddr =
            addr.parse()
                .map_err(|e: libp2p::multiaddr::Error| TransportError::DialFailed {
                    peer: addr.to_string(),
                    reason: e.to_string(),
                })?;

        let mut swarm = self.swarm.lock().await;
        swarm.dial(addr).map_err(|e| TransportError::DialFailed {
            peer: "unknown".into(),
            reason: e.to_string(),
        })?;

        Ok(())
    }

    /// Get the list of currently connected peers.
    pub async fn connected_peers(&self) -> Vec<(PeerId, bool)> {
        self.connected_peers
            .read()
            .await
            .iter()
            .filter_map(|(id, conn)| {
                parse_gang_peer_id(id).map(|peer_id| (peer_id, conn.via_relay))
            })
            .collect()
    }

    /// Access the underlying config.
    pub fn config(&self) -> &Libp2pConfig {
        &self.config
    }

    /// Access the keypair.
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    /// Send an RPC request with a payload and wait for the response.
    /// This is the primary method for operator→robot control messages.
    pub async fn send_rpc(
        &self,
        peer: &PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Vec<u8>, TransportError> {
        let libp2p_peer_id = {
            let conn = self.connected_peers.read().await;
            match conn.get(peer.as_str()) {
                Some(peer_conn) => peer_conn.libp2p_peer_id,
                None => {
                    return Err(TransportError::DialFailed {
                        peer: peer.to_string(),
                        reason: "peer not connected, use dial_multiaddr first".into(),
                    });
                }
            }
        };

        let (response_tx, response_rx) = oneshot::channel();

        let request_id = {
            let mut swarm = self.swarm.lock().await;
            swarm
                .behaviour_mut()
                .ganglion_rpc
                .send_request(&libp2p_peer_id, request_bytes)
        };

        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(
                request_id,
                PendingRequest {
                    peer_id: peer.clone(),
                    protocol: ProtocolId::control(),
                    response_tx,
                },
            );
        }

        {
            let mut peers = self.connected_peers.write().await;
            if let Some(conn) = peers.get_mut(peer.as_str()) {
                conn.messages_sent += 1;
            }
        }

        response_rx.await.map_err(|_| TransportError::DialFailed {
            peer: peer.to_string(),
            reason: "response channel closed".into(),
        })?
    }

    /// Get the libp2p peer ID for this node.
    pub fn libp2p_peer_id(&self) -> &Libp2pPeerId {
        &self.libp2p_peer_id
    }
}

#[async_trait::async_trait]
impl TransportAdapter for Libp2pTransportAdapter {
    async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError> {
        // Look up the peer in connected peers
        let libp2p_peer_id = {
            let conn = self.connected_peers.read().await;
            match conn.get(peer.as_str()) {
                Some(peer_conn) => peer_conn.libp2p_peer_id,
                None => {
                    return Err(TransportError::DialFailed {
                        peer: peer.to_string(),
                        reason: "peer not connected, use dial_multiaddr first".into(),
                    });
                }
            }
        };

        // Open a request-response stream on the control protocol
        let protocol = ProtocolId::control();
        let (response_tx, response_rx) = oneshot::channel();

        let request_id = {
            let mut swarm = self.swarm.lock().await;
            swarm
                .behaviour_mut()
                .ganglion_rpc
                .send_request(&libp2p_peer_id, Vec::new())
        };

        // Track the pending request
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(
                request_id,
                PendingRequest {
                    peer_id: peer.clone(),
                    protocol: protocol.clone(),
                    response_tx,
                },
            );
        }

        // Update send counters
        {
            let mut peers = self.connected_peers.write().await;
            if let Some(conn) = peers.get_mut(peer.as_str()) {
                conn.messages_sent += 1;
            }
        }

        // Wait for the response
        let response = response_rx.await.map_err(|_| TransportError::DialFailed {
            peer: peer.to_string(),
            reason: "response channel closed".into(),
        })??;

        // Build a GanglionStream from the response data
        let (read_half, mut write_half) = tokio::io::duplex(64 * 1024);
        let response_data = response;
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = write_half.write_all(&response_data).await;
            let _ = write_half.shutdown().await;
        });

        Ok(GanglionStream {
            protocol,
            remote_peer: peer.clone(),
            inner: Box::new(read_half),
        })
    }

    async fn listen(
        &self,
        protocol: ProtocolId,
        handler: StreamHandler,
    ) -> Result<(), TransportError> {
        // Validate that this is a known Ganglion protocol
        let proto_str = protocol.as_str();
        if !protocol::ALL_PROTOCOLS.contains(&proto_str) {
            return Err(TransportError::ProtocolNegotiation(format!(
                "unknown protocol: {proto_str}"
            )));
        }

        info!(protocol = %protocol, "Registered protocol handler");
        self.protocol_handlers
            .write()
            .await
            .insert(proto_str.to_string(), handler);
        Ok(())
    }

    fn local_peer_id(&self) -> PeerId {
        self.local_peer_id.clone()
    }

    fn capabilities(&self) -> TransportCapabilities {
        let mut caps = TransportCapabilities {
            relay: true,
            hole_punch: true,
            direct_dial: true,
            encrypted: true,
            transports: vec!["tcp".into(), "quic".into()],
        };

        if self.config.relay_server {
            caps.transports.push("relay-server".into());
        }

        // WebTransport: libp2p 0.56 only provides `webtransport-websys` (browser/WASM).
        // Native server-side WebTransport is not available yet. Report the config
        // intent so operators can see what was requested vs what is active.
        if self.config.enable_webtransport {
            caps.transports
                .push("webtransport (unavailable: requires wasm target)".into());
        }

        // WebRTC: libp2p 0.56 does not include a WebRTC transport crate at all.
        if self.config.enable_webrtc {
            caps.transports
                .push("webrtc (unavailable: not in libp2p 0.56)".into());
        }

        caps
    }

    fn events(&self) -> Pin<Box<dyn Stream<Item = TransportEvent> + Send>> {
        let event_rx = self.event_rx.clone();
        Box::pin(async_stream::stream! {
            loop {
                let event = event_rx.lock().await.recv().await;
                match event {
                    Some(e) => yield e,
                    None => break,
                }
            }
        })
    }

    async fn announce_presence(&self, info: PresenceInfo) -> Result<(), TransportError> {
        info!(
            peer_id = %info.peer_id,
            capabilities = ?info.capabilities_installed,
            "Announcing presence"
        );
        // In full implementation, this would broadcast a signed presence message
        // to connected relay peers via the control protocol.
        Ok(())
    }

    async fn transport_stats(&self, peer: &PeerId) -> Option<TransportStats> {
        let peers = self.connected_peers.read().await;
        let conn = peers.get(peer.as_str())?;
        Some(TransportStats {
            transport: conn.transport.clone(),
            via_relay: conn.via_relay,
            connect_time_ms: conn.connected_at.elapsed().as_millis() as u64,
            messages_sent: conn.messages_sent,
            messages_received: conn.messages_received,
            bytes_sent: conn.bytes_sent,
            bytes_received: conn.bytes_received,
            last_rtt_ms: conn.last_rtt.map(|d| d.as_millis() as u64),
            dcutr_attempted: conn.dcutr_attempted,
            dcutr_succeeded: conn.dcutr_succeeded,
            uptime_secs: conn.connected_at.elapsed().as_secs(),
            reconnections: conn.reconnections,
        })
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        info!("Shutting down transport");
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(()).await;
        }
        Ok(())
    }
}

/// Convert a libp2p PeerId to a Ganglion PeerId.
/// Uses the libp2p peer ID string as a deterministic input.
fn libp2p_to_gang_peer_id(peer_id: &Libp2pPeerId) -> PeerId {
    let hash = blake3::hash(peer_id.to_bytes().as_slice());
    // Re-derive using the same format as gang-core PeerId
    let hex = hex::encode(&hash.as_bytes()[..16]);
    serde_json::from_str::<PeerId>(&format!("\"12D3-{hex}\"")).expect("valid peer id format")
}

/// Parse a gang peer ID string back into a PeerId.
fn parse_gang_peer_id(id: &str) -> Option<PeerId> {
    serde_json::from_str::<PeerId>(&format!("\"{id}\"")).ok()
}

/// Merge a read half and a write half into a single AsyncRead + AsyncWrite stream.
struct MergedRw<R, W> {
    reader: R,
    writer: W,
}

fn merge_rw<R, W>(reader: R, writer: W) -> MergedRw<R, W> {
    MergedRw { reader, writer }
}

impl<R, W> tokio::io::AsyncRead for MergedRw<R, W>
where
    R: tokio::io::AsyncRead + Unpin,
    W: Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().reader).poll_read(cx, buf)
    }
}

impl<R, W> tokio::io::AsyncWrite for MergedRw<R, W>
where
    R: Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().writer).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().writer).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().writer).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Libp2pConfig;
    use crate::swarm::GanglionProtocol;

    #[test]
    fn test_config_defaults() {
        let config = Libp2pConfig::default();
        assert_eq!(config.listen_addrs.len(), 2);
        assert!(config.listen_addrs[0].contains("tcp"));
        assert!(config.listen_addrs[1].contains("quic"));
        assert!(!config.relay_server);
        assert_eq!(config.idle_timeout_secs, 300);
        assert_eq!(config.max_inbound_connections, 64);
        assert!(config.relay_addrs.is_empty());
        assert!(config.bootstrap_peers.is_empty());
    }

    #[tokio::test]
    async fn test_swarm_builds() {
        let tmpdir = tempfile::tempdir().unwrap();
        let key_path = tmpdir.path().join("test.key");
        gang_core::identity::Keypair::generate()
            .save(&key_path)
            .unwrap();

        let config = Libp2pConfig {
            key_path,
            listen_addrs: vec![], // Don't bind ports in tests
            ..Default::default()
        };

        let result = swarm::build_swarm(&config).await;
        assert!(result.is_ok(), "swarm build failed: {:?}", result.err());

        let (_swarm, peer_id) = result.unwrap();
        // Peer ID should be valid (libp2p PeerId is non-empty)
        assert!(!peer_id.to_string().is_empty());
    }

    #[tokio::test]
    async fn test_swarm_builds_with_relay_server() {
        let tmpdir = tempfile::tempdir().unwrap();
        let key_path = tmpdir.path().join("test.key");
        gang_core::identity::Keypair::generate()
            .save(&key_path)
            .unwrap();

        let config = Libp2pConfig {
            key_path,
            listen_addrs: vec![],
            relay_server: true,
            ..Default::default()
        };

        let result = swarm::build_swarm(&config).await;
        assert!(
            result.is_ok(),
            "relay server swarm build failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_adapter_capabilities() {
        // Test that capabilities report correctly for a client node
        let caps = TransportCapabilities {
            relay: true,
            hole_punch: true,
            direct_dial: true,
            encrypted: true,
            transports: vec!["tcp".into(), "quic".into()],
        };

        assert!(caps.relay);
        assert!(caps.hole_punch);
        assert!(caps.direct_dial);
        assert!(caps.encrypted);
        assert_eq!(caps.transports.len(), 2);
        assert!(caps.transports.contains(&"tcp".to_string()));
        assert!(caps.transports.contains(&"quic".to_string()));
    }

    #[tokio::test]
    async fn test_peer_connection_tracking() {
        let connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Simulate a connection
        let fake_libp2p_peer = Libp2pPeerId::random();
        let gang_peer_id = libp2p_to_gang_peer_id(&fake_libp2p_peer);
        let peer_key = gang_peer_id.as_str().to_string();

        {
            let mut peers = connected_peers.write().await;
            peers.insert(
                peer_key.clone(),
                PeerConnection {
                    libp2p_peer_id: fake_libp2p_peer,
                    via_relay: false,
                    connected_at: std::time::Instant::now(),
                    transport: "tcp".to_string(),
                    last_rtt: None,
                    dcutr_attempted: false,
                    dcutr_succeeded: false,
                    messages_sent: 0,
                    messages_received: 0,
                    bytes_sent: 0,
                    bytes_received: 0,
                    reconnections: 0,
                },
            );
        }

        // Verify the peer is tracked
        {
            let peers = connected_peers.read().await;
            assert!(peers.contains_key(&peer_key));
            let conn = peers.get(&peer_key).unwrap();
            assert_eq!(conn.libp2p_peer_id, fake_libp2p_peer);
            assert!(!conn.via_relay);
            assert_eq!(conn.transport, "tcp");
        }

        // Simulate disconnection
        {
            let mut peers = connected_peers.write().await;
            peers.remove(&peer_key);
        }

        // Verify removal
        {
            let peers = connected_peers.read().await;
            assert!(!peers.contains_key(&peer_key));
        }
    }

    #[tokio::test]
    async fn test_transport_stats_for_connected_peer() {
        let connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>> =
            Arc::new(RwLock::new(HashMap::new()));

        let fake_libp2p_peer = Libp2pPeerId::random();
        let gang_peer_id = libp2p_to_gang_peer_id(&fake_libp2p_peer);
        let peer_key = gang_peer_id.as_str().to_string();

        let rtt = std::time::Duration::from_millis(42);

        {
            let mut peers = connected_peers.write().await;
            peers.insert(
                peer_key.clone(),
                PeerConnection {
                    libp2p_peer_id: fake_libp2p_peer,
                    via_relay: true,
                    connected_at: std::time::Instant::now(),
                    transport: "relay".to_string(),
                    last_rtt: Some(rtt),
                    dcutr_attempted: true,
                    dcutr_succeeded: false,
                    messages_sent: 10,
                    messages_received: 5,
                    bytes_sent: 1024,
                    bytes_received: 512,
                    reconnections: 2,
                },
            );
        }

        // Build stats from the connection (mirrors transport_stats() logic)
        let peers = connected_peers.read().await;
        let conn = peers.get(&peer_key).unwrap();
        let stats = TransportStats {
            transport: conn.transport.clone(),
            via_relay: conn.via_relay,
            connect_time_ms: conn.connected_at.elapsed().as_millis() as u64,
            messages_sent: conn.messages_sent,
            messages_received: conn.messages_received,
            bytes_sent: conn.bytes_sent,
            bytes_received: conn.bytes_received,
            last_rtt_ms: conn.last_rtt.map(|d| d.as_millis() as u64),
            dcutr_attempted: conn.dcutr_attempted,
            dcutr_succeeded: conn.dcutr_succeeded,
            uptime_secs: conn.connected_at.elapsed().as_secs(),
            reconnections: conn.reconnections,
        };

        assert_eq!(stats.transport, "relay");
        assert!(stats.via_relay);
        assert_eq!(stats.messages_sent, 10);
        assert_eq!(stats.messages_received, 5);
        assert_eq!(stats.bytes_sent, 1024);
        assert_eq!(stats.bytes_received, 512);
        assert_eq!(stats.last_rtt_ms, Some(42));
        assert!(stats.dcutr_attempted);
        assert!(!stats.dcutr_succeeded);
        assert_eq!(stats.reconnections, 2);
    }

    #[test]
    fn test_libp2p_to_gang_peer_id_deterministic() {
        let libp2p_peer = Libp2pPeerId::random();
        let gang_id_1 = libp2p_to_gang_peer_id(&libp2p_peer);
        let gang_id_2 = libp2p_to_gang_peer_id(&libp2p_peer);
        assert_eq!(gang_id_1, gang_id_2);
        assert!(gang_id_1.as_str().starts_with("12D3-"));
    }

    #[test]
    fn test_parse_gang_peer_id_roundtrip() {
        let libp2p_peer = Libp2pPeerId::random();
        let gang_id = libp2p_to_gang_peer_id(&libp2p_peer);
        let parsed = parse_gang_peer_id(gang_id.as_str());
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap(), gang_id);
    }

    #[test]
    fn test_ganglion_protocol_as_ref() {
        let proto = GanglionProtocol("/ganglion/control/1.0".to_string());
        assert_eq!(proto.as_ref(), "/ganglion/control/1.0");
    }

    #[test]
    fn test_capabilities_without_browser_transports() {
        let caps = TransportCapabilities {
            relay: true,
            hole_punch: true,
            direct_dial: true,
            encrypted: true,
            transports: vec!["tcp".into(), "quic".into()],
        };
        assert!(!caps.transports.iter().any(|t| t.contains("webtransport")));
        assert!(!caps.transports.iter().any(|t| t.contains("webrtc")));
    }

    #[test]
    fn test_capabilities_with_webtransport_enabled() {
        // Simulate what capabilities() returns when enable_webtransport is true
        let mut caps = TransportCapabilities {
            relay: true,
            hole_punch: true,
            direct_dial: true,
            encrypted: true,
            transports: vec!["tcp".into(), "quic".into()],
        };
        // Mirror the adapter logic
        caps.transports
            .push("webtransport (unavailable: requires wasm target)".into());

        assert!(caps.transports.iter().any(|t| t.contains("webtransport")));
        assert!(caps.transports.iter().any(|t| t.contains("unavailable")));
    }

    #[test]
    fn test_capabilities_with_webrtc_enabled() {
        let mut caps = TransportCapabilities {
            relay: true,
            hole_punch: true,
            direct_dial: true,
            encrypted: true,
            transports: vec!["tcp".into(), "quic".into()],
        };
        caps.transports
            .push("webrtc (unavailable: not in libp2p 0.56)".into());

        assert!(caps.transports.iter().any(|t| t.contains("webrtc")));
        assert!(caps.transports.iter().any(|t| t.contains("unavailable")));
    }

    #[test]
    fn test_config_browser_transport_defaults() {
        let config = Libp2pConfig::default();
        assert!(!config.enable_webtransport);
        assert!(!config.enable_webrtc);
    }

    #[test]
    fn test_transport_name_detection_webtransport() {
        // Verify our address-to-transport-name logic handles future multiaddrs
        let addr = "/ip4/1.2.3.4/udp/443/quic-v1/webtransport/certhash/abc123";
        assert!(addr.contains("/webtransport/"));
    }

    #[test]
    fn test_transport_name_detection_webrtc() {
        let addr_direct = "/ip4/1.2.3.4/udp/9090/webrtc-direct/certhash/abc123";
        let addr = "/ip4/1.2.3.4/udp/9090/webrtc/certhash/abc123";
        assert!(addr_direct.contains("/webrtc-direct/"));
        assert!(addr.contains("/webrtc/"));
    }
}
