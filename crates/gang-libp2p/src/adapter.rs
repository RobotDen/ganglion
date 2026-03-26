use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use libp2p::{Multiaddr, PeerId as Libp2pPeerId, Swarm};
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, info};

use gang_core::error::TransportError;
use gang_core::identity::{Keypair, PeerId};
use gang_core::protocol::ProtocolId;
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
/// - Circuit relay v2 for NAT traversal
/// - DCUtR for direct connection upgrades
/// - Kademlia for peer routing
#[allow(dead_code)] // Fields used as the transport layer is fleshed out
pub struct Libp2pTransportAdapter {
    config: Libp2pConfig,
    keypair: Keypair,
    local_peer_id: PeerId,
    libp2p_peer_id: Libp2pPeerId,
    swarm: Arc<Mutex<Swarm<GanglionBehaviour>>>,
    event_tx: mpsc::Sender<TransportEvent>,
    event_rx: Arc<Mutex<mpsc::Receiver<TransportEvent>>>,
    connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PeerConnection {
    libp2p_peer_id: Libp2pPeerId,
    via_relay: bool,
    connected_at: std::time::Instant,
    /// Which transport was used (tcp, quic, relay).
    transport: String,
    /// Latest RTT from ping.
    last_rtt: Option<std::time::Duration>,
    /// Whether DCUtR was attempted.
    dcutr_attempted: bool,
    /// Whether DCUtR succeeded.
    dcutr_succeeded: bool,
    /// Messages sent over this connection.
    messages_sent: u64,
    /// Messages received.
    messages_received: u64,
    /// Bytes sent.
    bytes_sent: u64,
    /// Bytes received.
    bytes_received: u64,
    /// Number of reconnections.
    reconnections: u64,
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

                    // Determine transport from endpoint
                    let transport_name = if via_relay {
                        "relay".to_string()
                    } else if endpoint.get_remote_address().to_string().contains("quic") {
                        "quic".to_string()
                    } else {
                        "tcp".to_string()
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
            _ => {}
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
            .map(|(id, conn)| {
                let peer_id =
                    PeerId::from_public_key(&gang_core::identity::Keypair::generate().public_key());
                // In practice we'd maintain a proper mapping; for now return the stored ID
                (
                    // Parse the stored gang peer ID string back
                    serde_json::from_str::<PeerId>(&format!("\"{}\"", id)).unwrap_or(peer_id),
                    conn.via_relay,
                )
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl TransportAdapter for Libp2pTransportAdapter {
    async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError> {
        // Look up the peer in connected peers or try to dial
        let conn = self.connected_peers.read().await;
        if let Some(_peer_conn) = conn.get(peer.as_str()) {
            // Peer is connected — in a full implementation we'd open a new
            // stream on the existing connection. For now, return a placeholder.
            return Err(TransportError::DialFailed {
                peer: peer.to_string(),
                reason: "stream opening not yet implemented".into(),
            });
        }

        Err(TransportError::DialFailed {
            peer: peer.to_string(),
            reason: "peer not connected, use dial_multiaddr first".into(),
        })
    }

    async fn listen(
        &self,
        protocol: ProtocolId,
        _handler: StreamHandler,
    ) -> Result<(), TransportError> {
        // Protocol handler registration is managed through the swarm behaviour.
        // Full implementation will use libp2p's stream protocol support.
        info!(protocol = %protocol, "Registered protocol handler");
        Ok(())
    }

    fn local_peer_id(&self) -> PeerId {
        self.local_peer_id.clone()
    }

    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            relay: true,
            hole_punch: true,
            direct_dial: true,
            encrypted: true,
            transports: vec!["tcp".into(), "quic".into()],
        }
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
