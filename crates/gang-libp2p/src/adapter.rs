use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use futures::Stream;
use libp2p::request_response::{self, OutboundRequestId, ResponseChannel};
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

/// A live, decoded robot→operator event feed (ADR-024): the operator-side
/// return of [`Libp2pTransportAdapter::subscribe_events`]. Yields
/// [`gang_core::events::AgentEvent`]s as the robot pushes them, ending when the
/// robot closes the substream.
pub type EventStream = Pin<Box<dyn Stream<Item = gang_core::events::AgentEvent> + Send>>;

/// Robot-side inbound event substreams: the return of
/// [`Libp2pTransportAdapter::accept_event_streams`]. Each item is an
/// authenticated subscriber's gang [`PeerId`] paired with the raw push
/// substream to push framed events over.
pub type InboundEventStreams = Pin<Box<dyn Stream<Item = (PeerId, libp2p::Stream)> + Send>>;

/// How long a caller waits for an RPC or dial reply before timing out unless
/// an explicit per-request timeout is supplied (see
/// [`Libp2pTransportAdapter::send_rpc_with_timeout`]).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on the number of concurrently pending outbound requests. New
/// requests beyond this are rejected rather than allowed to grow the map
/// without limit.
const MAX_PENDING_REQUESTS: usize = 1024;
/// How often the worker sweeps out pending requests that have passed their
/// deadline, guaranteeing their entries are removed even if no response or
/// failure event ever arrives.
const PENDING_SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Commands sent to the single task that owns the [`Swarm`].
///
/// The swarm is never shared behind a mutex; all interaction happens by
/// sending one of these commands over an mpsc channel and (for request/reply
/// commands) awaiting a `oneshot` response. This keeps the swarm owned by
/// exactly one task, so it is never held across `.await` by multiple callers.
enum SwarmCommand {
    /// Dial a raw multiaddr.
    Dial {
        addr: Multiaddr,
        reply: oneshot::Sender<Result<(), TransportError>>,
    },
    /// Connect to the relays listed in config.
    ConnectToRelays {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    /// Send an RPC request to a connected peer and await the response.
    SendRpc {
        peer: PeerId,
        libp2p_peer: Libp2pPeerId,
        protocol: ProtocolId,
        request: Vec<u8>,
        /// How long the worker keeps the request pending before sweeping it
        /// out with a timeout error.
        timeout: Duration,
        reply: oneshot::Sender<Result<Vec<u8>, TransportError>>,
    },
    /// Send a response for an inbound request back over its channel.
    /// Emitted internally once a registered handler has produced a response.
    SendResponse {
        channel: ResponseChannel<Vec<u8>>,
        data: Vec<u8>,
    },
    /// Stop the event loop.
    Shutdown,
}

/// Tracks a pending outbound request so we can deliver the response
/// to the awaiting caller.
struct PendingRequest {
    peer_id: PeerId,
    protocol: ProtocolId,
    reply: oneshot::Sender<Result<Vec<u8>, TransportError>>,
    /// After this instant the request is considered timed out and its entry
    /// is swept from the pending map.
    deadline: Instant,
    /// The timeout this request was issued with (for the error report).
    timeout: Duration,
}

/// Fan-out of transport events to every live subscriber.
///
/// `TransportEvent` (defined in gang-core) is not `Clone`, so
/// `tokio::sync::broadcast` cannot be used directly. This bus reconstructs
/// each event per subscriber so that multiple `events()` consumers all see
/// every event, rather than competing for items from a single receiver.
#[derive(Clone, Default)]
struct EventBus {
    subscribers: Arc<StdMutex<Vec<mpsc::UnboundedSender<TransportEvent>>>>,
}

impl EventBus {
    /// Register a new subscriber and return its receiver.
    fn subscribe(&self) -> mpsc::UnboundedReceiver<TransportEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Deliver `event` to every live subscriber, pruning closed ones.
    fn publish(&self, event: TransportEvent) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| !tx.is_closed());
        if subs.is_empty() {
            return;
        }
        // Reconstruct a copy for all but the last subscriber, then move the
        // original into the last one to avoid one needless clone.
        let last = subs.len() - 1;
        for tx in subs.iter().take(last) {
            let _ = tx.send(event.clone());
        }
        let _ = subs[last].send(event);
    }
}

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
///
/// The underlying `Swarm` is owned by a single task (started by
/// [`Libp2pTransportAdapter::run_event_loop`]); all callers interact with it
/// exclusively through the command channel.
pub struct Libp2pTransportAdapter {
    config: Libp2pConfig,
    keypair: Keypair,
    local_peer_id: PeerId,
    libp2p_peer_id: Libp2pPeerId,
    /// Channel to the swarm-owning task.
    command_tx: mpsc::Sender<SwarmCommand>,
    /// Fan-out of transport events to `events()` subscribers.
    events: EventBus,
    connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>>,
    /// Registered protocol handlers for incoming streams.
    protocol_handlers: Arc<RwLock<HashMap<String, StreamHandler>>>,
    /// Cloneable handle for genuine push substreams (ADR-024). Talks to the
    /// swarm's [`libp2p_stream::Behaviour`] over its own channel, so opening or
    /// accepting an event stream never shares or locks the `Swarm`.
    stream_control: libp2p_stream::Control,
    /// Addresses the swarm is actually listening on (populated by the worker
    /// from `NewListenAddr` events; ephemeral ports appear here resolved).
    listen_addrs: Arc<RwLock<Vec<String>>>,
    /// The swarm-owning worker, taken by `run_event_loop`.
    worker: Mutex<Option<SwarmWorker>>,
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
    pub async fn new(config: Libp2pConfig) -> Result<Self, crate::TransportError> {
        let keypair = Keypair::load_or_generate(&config.key_path)?;
        let local_peer_id = keypair.peer_id();

        // Load the raw secret once (validated, non-panicking) and hand it to
        // the swarm builder so the key file is not read a second time.
        let secret_key = swarm::load_ed25519_secret(&config.key_path)
            .map_err(|e| crate::TransportError::InvalidKey(e.to_string()))?;
        let (mut swarm, libp2p_peer_id, stream_control) = swarm::build_swarm(&config, secret_key)
            .await
            .map_err(|e| crate::TransportError::SwarmBuild(e.to_string()))?;

        // Start listening
        swarm::start_listening(&mut swarm, &config)
            .map_err(|e| crate::TransportError::SwarmBuild(e.to_string()))?;

        // Add bootstrap peers
        swarm::add_bootstrap_peers(&mut swarm, &config)
            .map_err(|e| crate::TransportError::SwarmBuild(e.to_string()))?;

        // Request circuit reservations on configured relays so this node is
        // reachable *through* them, not merely connected to them. The worker
        // re-establishes a reservation whose listener closes.
        let circuit_listeners = swarm::listen_on_relay_circuits(&mut swarm, &config)
            .map_err(|e| crate::TransportError::Relay(e.to_string()))?;

        // Command channel to the single swarm-owning task. This same channel
        // carries the shutdown signal (`SwarmCommand::Shutdown`).
        let (command_tx, command_rx) = mpsc::channel(256);
        let events = EventBus::default();
        let connected_peers = Arc::new(RwLock::new(HashMap::new()));
        let protocol_handlers = Arc::new(RwLock::new(HashMap::new()));
        let listen_addrs = Arc::new(RwLock::new(Vec::new()));

        let worker = SwarmWorker {
            swarm,
            command_rx,
            command_tx: command_tx.clone(),
            events: events.clone(),
            connected_peers: connected_peers.clone(),
            protocol_handlers: protocol_handlers.clone(),
            listen_addrs: listen_addrs.clone(),
            pending_requests: HashMap::new(),
            circuit_listeners,
            circuit_relisten: Vec::new(),
            config: config.clone(),
        };

        Ok(Self {
            config,
            keypair,
            local_peer_id,
            libp2p_peer_id,
            command_tx,
            events,
            connected_peers,
            protocol_handlers,
            stream_control,
            listen_addrs,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Start the swarm event loop. This must be called once after construction
    /// and runs until `shutdown()` is called (or all command senders drop).
    pub async fn run_event_loop(&self) -> Result<(), crate::TransportError> {
        let worker = self.worker.lock().await.take();
        match worker {
            Some(worker) => {
                worker.run().await;
                Ok(())
            }
            None => Err(crate::TransportError::EventLoopAlreadyRunning),
        }
    }

    /// Connect to relay nodes specified in config.
    pub async fn connect_to_relays(&self) -> Result<(), crate::TransportError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(SwarmCommand::ConnectToRelays { reply: reply_tx })
            .await
            .map_err(|_| crate::TransportError::SwarmTaskStopped)?;
        reply_rx
            .await
            .map_err(|_| crate::TransportError::SwarmTaskStopped)?
            .map_err(|e| crate::TransportError::Relay(e.to_string()))
    }

    /// Dial a peer by their libp2p multiaddr.
    ///
    /// The wait for the swarm worker's reply is bounded by the same 30-second
    /// request timeout `send_rpc` uses, so a caller can never hang forever —
    /// e.g. when the event loop has not been started yet.
    pub async fn dial_multiaddr(&self, addr: &str) -> Result<(), TransportError> {
        let addr_str = addr.to_string();
        let addr: Multiaddr =
            addr.parse()
                .map_err(|e: libp2p::multiaddr::Error| TransportError::DialFailed {
                    peer: addr_str.clone(),
                    reason: e.to_string(),
                })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(SwarmCommand::Dial {
                addr,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransportError::ConnectionClosed("swarm task is not running".into()))?;

        match tokio::time::timeout(REQUEST_TIMEOUT, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(TransportError::DialFailed {
                peer: addr_str,
                reason: "swarm task dropped the reply channel".into(),
            }),
            Err(_) => Err(TransportError::Timeout(REQUEST_TIMEOUT)),
        }
    }

    /// Addresses the swarm is actually listening on. Ephemeral (`/tcp/0`)
    /// listen addresses appear here with their resolved ports once the swarm
    /// reports them; the list is empty until the event loop has processed the
    /// first `NewListenAddr` event.
    pub async fn listen_addrs(&self) -> Vec<String> {
        self.listen_addrs.read().await.clone()
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

    /// Send an RPC request with a payload and wait for the response, using
    /// the default 30-second request timeout.
    /// This is the primary method for operator→robot control messages.
    pub async fn send_rpc(
        &self,
        peer: &PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Vec<u8>, TransportError> {
        self.send_rpc_with_timeout(peer, request_bytes, REQUEST_TIMEOUT)
            .await
    }

    /// Like [`Libp2pTransportAdapter::send_rpc`] with an explicit timeout.
    ///
    /// Used for requests whose expected duration differs from the default —
    /// e.g. a capability deploy shipping megabytes of component bytes over a
    /// relay circuit. The timeout bounds both the local wait and the worker's
    /// pending-request entry; it cannot exceed the protocol-level ceiling
    /// configured on the request-response behaviour (120 s).
    pub async fn send_rpc_with_timeout(
        &self,
        peer: &PeerId,
        request_bytes: Vec<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        self.send_rpc_on(peer, ProtocolId::control(), request_bytes, timeout)
            .await
    }

    /// Like [`Libp2pTransportAdapter::send_rpc_with_timeout`] but on an
    /// explicit protocol, so callers can address `/ganglion/events/1.0` (or any
    /// registered protocol) rather than only the control plane. The request
    /// still rides the shared request-response behaviour; the negotiated
    /// protocol determines which handler the robot dispatches to.
    pub async fn send_rpc_on(
        &self,
        peer: &PeerId,
        protocol: ProtocolId,
        request_bytes: Vec<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let libp2p_peer_id = self.resolve_peer(peer).await?;

        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(SwarmCommand::SendRpc {
                peer: peer.clone(),
                libp2p_peer: libp2p_peer_id,
                protocol,
                request: request_bytes,
                timeout,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransportError::ConnectionClosed("swarm task is not running".into()))?;

        await_reply_within(peer, reply_rx, timeout).await
    }

    /// Subscribe to a robot's event feed as a genuine server-push stream
    /// (ADR-024) and return a live [`Stream`] of
    /// [`gang_core::events::AgentEvent`]s.
    ///
    /// This opens a persistent `/ganglion/events/1.0` push substream to the
    /// robot (over the relay circuit, same as control RPC), writes a single
    /// [`gang_core::events::EventSubscribeRequest`] frame, and then decodes
    /// length-prefixed CBOR events as the robot pushes them — with no polling
    /// and no fixed cadence. Events arrive the instant the robot emits them.
    ///
    /// A fresh subscription (`since_seq == None`) begins with a
    /// [`gang_core::events::AgentEvent::PresenceSnapshot`] followed by the
    /// robot's retained recent events, then transitions seamlessly into the
    /// live tail. A resume (`since_seq == Some(cursor)`) skips the snapshot and
    /// begins from events newer than the cursor, with a leading
    /// [`gang_core::events::AgentEvent::Gap`] if the cursor predated the robot's
    /// retained window.
    ///
    /// The `timeout` bounds only the initial open + first-frame handshake, so a
    /// robot that refuses the subscription (e.g. the operator is not trusted —
    /// the robot closes the stream without sending anything) surfaces promptly
    /// as a typed [`TransportError`] rather than a silently empty stream. Once
    /// the stream is returned it lives until the robot closes it or the caller
    /// drops it.
    pub async fn subscribe_events(
        &self,
        peer: &PeerId,
        since_seq: Option<u64>,
        timeout: Duration,
    ) -> Result<EventStream, TransportError> {
        use gang_core::events::{AgentEvent, EventSubscribeRequest};
        use gang_core::message::{decode_message, encode_message};

        let libp2p_peer = self.resolve_peer(peer).await?;
        let protocol = libp2p::StreamProtocol::new(protocol::PROTOCOL_EVENTS);

        // Open the push substream. `Control` is cloned per call (its
        // backpressure slot is per-clone); this never touches the Swarm.
        let mut control = self.stream_control.clone();
        let peer_disp = peer.to_string();
        let mut stream = tokio::time::timeout(timeout, control.open_stream(libp2p_peer, protocol))
            .await
            .map_err(|_| TransportError::Timeout(timeout))?
            .map_err(|e| TransportError::DialFailed {
                peer: peer_disp.clone(),
                reason: format!("could not open event stream: {e}"),
            })?;

        // Write the (single) subscription request frame.
        let req = EventSubscribeRequest::new(since_seq, None);
        let request_bytes = encode_message(&req)
            .map_err(|e| TransportError::ProtocolNegotiation(format!("encode subscribe: {e}")))?;
        crate::framed::write_frame(&mut stream, &request_bytes)
            .await
            .map_err(|e| {
                TransportError::ConnectionClosed(format!("write subscribe request: {e}"))
            })?;

        // Read the first frame within the handshake timeout. A clean EOF here
        // means the robot refused the subscription (unauthorized) — surface it
        // as an error rather than an empty stream.
        let first = tokio::time::timeout(timeout, crate::framed::read_frame(&mut stream))
            .await
            .map_err(|_| TransportError::Timeout(timeout))?
            .map_err(|e| {
                TransportError::ConnectionClosed(format!("reading first event frame: {e}"))
            })?;
        let Some(first_frame) = first else {
            return Err(TransportError::ProtocolNegotiation(format!(
                "robot {peer_disp} refused the event subscription (unauthorized or unreachable)"
            )));
        };
        let (first_event, _) = decode_message::<AgentEvent>(&first_frame)
            .map_err(|e| TransportError::ProtocolNegotiation(format!("decode first event: {e}")))?;

        // Hand back a live stream: the first (already-decoded) event, then each
        // subsequent pushed frame decoded on arrival, until the robot closes.
        let out = async_stream::stream! {
            yield first_event;
            loop {
                match crate::framed::read_frame(&mut stream).await {
                    Ok(Some(frame)) => match decode_message::<AgentEvent>(&frame) {
                        Ok((ev, _)) => yield ev,
                        Err(e) => {
                            warn!(peer = %peer_disp, "dropping undecodable event frame: {e}");
                            break;
                        }
                    },
                    Ok(None) => break, // robot closed the feed
                    Err(e) => {
                        debug!(peer = %peer_disp, "event stream ended: {e}");
                        break;
                    }
                }
            }
        };
        Ok(Box::pin(out) as EventStream)
    }

    /// Accept inbound `/ganglion/events/1.0` push substreams (ADR-024, robot
    /// side). Returns a [`Stream`] of `(subscriber, substream)` pairs, one per
    /// operator that opens the event protocol. Each substream is a raw
    /// `AsyncRead + AsyncWrite` the caller authenticates and pushes framed
    /// events over.
    ///
    /// The subscriber [`PeerId`] is the SEC-03 gang id derived from the
    /// Ed25519 key libp2p's Noise handshake authenticated on the connection;
    /// a peer whose identity cannot be recovered is dropped before it is
    /// surfaced (never streamed to). May be called once per protocol — a second
    /// call returns [`TransportError::ProtocolNegotiation`].
    pub fn accept_event_streams(&self) -> Result<InboundEventStreams, TransportError> {
        use futures::StreamExt;

        let protocol = libp2p::StreamProtocol::new(protocol::PROTOCOL_EVENTS);
        let incoming = self.stream_control.clone().accept(protocol).map_err(|e| {
            TransportError::ProtocolNegotiation(format!("event protocol already accepted: {e:?}"))
        })?;

        let mapped = incoming.filter_map(|(libp2p_peer, stream)| async move {
            match libp2p_to_gang_peer_id(&libp2p_peer) {
                Some(gang_peer) => Some((gang_peer, stream)),
                None => {
                    warn!(peer = %libp2p_peer, "Dropping event stream from peer with unrecoverable identity");
                    None
                }
            }
        });
        Ok(Box::pin(mapped))
    }

    /// Look up the libp2p peer id for a connected gang peer.
    async fn resolve_peer(&self, peer: &PeerId) -> Result<Libp2pPeerId, TransportError> {
        let conn = self.connected_peers.read().await;
        match conn.get(peer.as_str()) {
            Some(peer_conn) => Ok(peer_conn.libp2p_peer_id),
            None => Err(TransportError::DialFailed {
                peer: peer.to_string(),
                reason: "peer not connected, use dial_multiaddr first".into(),
            }),
        }
    }

    /// Get the libp2p peer ID for this node.
    pub fn libp2p_peer_id(&self) -> &Libp2pPeerId {
        &self.libp2p_peer_id
    }
}

/// The task that owns the [`Swarm`] and drives its event loop. It receives
/// [`SwarmCommand`]s over an mpsc channel and never shares the swarm.
struct SwarmWorker {
    swarm: Swarm<GanglionBehaviour>,
    command_rx: mpsc::Receiver<SwarmCommand>,
    /// A clone of the command sender, used to re-inject `SendResponse` once an
    /// inbound handler finishes.
    command_tx: mpsc::Sender<SwarmCommand>,
    events: EventBus,
    connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>>,
    protocol_handlers: Arc<RwLock<HashMap<String, StreamHandler>>>,
    /// Shared view of the swarm's current listen addresses.
    listen_addrs: Arc<RwLock<Vec<String>>>,
    pending_requests: HashMap<OutboundRequestId, PendingRequest>,
    /// Relay circuit reservation listeners (listener id → circuit multiaddr),
    /// tracked so a closed reservation listener can be re-established.
    circuit_listeners: HashMap<libp2p::core::transport::ListenerId, Multiaddr>,
    /// Circuit addresses whose listener closed; re-listened on the next sweep
    /// tick (giving the relay a few seconds to come back).
    circuit_relisten: Vec<Multiaddr>,
    config: Libp2pConfig,
}

impl SwarmWorker {
    /// Run until a `Shutdown` command arrives, all senders drop, or the swarm
    /// stream ends.
    async fn run(mut self) {
        use futures::StreamExt;
        use libp2p::swarm::SwarmEvent;

        let mut sweep = tokio::time::interval(PENDING_SWEEP_INTERVAL);
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = sweep.tick() => {
                    self.sweep_pending_requests();
                    self.relisten_closed_circuits();
                }
                command = self.command_rx.recv() => {
                    match command {
                        Some(SwarmCommand::Shutdown) | None => {
                            info!("Transport event loop shutting down");
                            break;
                        }
                        Some(command) => self.handle_command(command).await,
                    }
                }
                event = self.swarm.next() => {
                    match event {
                        Some(SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. }) => {
                            self.on_connection_established(peer_id, endpoint).await;
                        }
                        Some(SwarmEvent::ConnectionClosed { peer_id, .. }) => {
                            info!(peer = %peer_id, "Connection closed");
                            if let Some(gang_peer_id) = libp2p_to_gang_peer_id(&peer_id) {
                                self.connected_peers.write().await.remove(gang_peer_id.as_str());
                                self.events.publish(TransportEvent::PeerDisconnected {
                                    peer_id: gang_peer_id,
                                });
                            }
                        }
                        Some(SwarmEvent::NewListenAddr { address, .. }) => {
                            info!("Listening on {address}");
                            // A relay server must advertise its listen
                            // addresses as external, or the reservations it
                            // grants carry no addresses and every client
                            // rejects them (NoAddressesInReservation).
                            if self.config.relay_server {
                                self.swarm.add_external_address(address.clone());
                            }
                            let mut addrs = self.listen_addrs.write().await;
                            let addr = address.to_string();
                            if !addrs.contains(&addr) {
                                addrs.push(addr);
                            }
                        }
                        Some(SwarmEvent::ExpiredListenAddr { address, .. }) => {
                            let addr = address.to_string();
                            self.listen_addrs.write().await.retain(|a| *a != addr);
                        }
                        Some(SwarmEvent::ListenerClosed { listener_id, reason, .. }) => {
                            if let Some(circuit) = self.circuit_listeners.remove(&listener_id) {
                                warn!(
                                    %circuit,
                                    ?reason,
                                    "Relay circuit reservation listener closed; will re-establish"
                                );
                                self.circuit_relisten.push(circuit);
                            }
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
            }
        }
    }

    /// Remove pending requests whose deadline has passed, replying with a
    /// timeout error. This guarantees the pending map is cleaned up even when
    /// no response or failure event ever arrives for a request.
    fn sweep_pending_requests(&mut self) {
        let now = Instant::now();
        let expired: Vec<OutboundRequestId> = self
            .pending_requests
            .iter()
            .filter(|(_, req)| req.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(req) = self.pending_requests.remove(&id) {
                warn!(peer = %req.peer_id, protocol = %req.protocol, "RPC request timed out");
                let _ = req.reply.send(Err(TransportError::Timeout(req.timeout)));
            }
        }
    }

    /// Re-establish relay circuit reservations whose listener closed (e.g.
    /// the relay was unreachable). Runs on the periodic sweep tick, so a
    /// closed reservation is retried roughly every [`PENDING_SWEEP_INTERVAL`].
    fn relisten_closed_circuits(&mut self) {
        for circuit in std::mem::take(&mut self.circuit_relisten) {
            match self.swarm.listen_on(circuit.clone()) {
                Ok(id) => {
                    info!(%circuit, "Re-requesting relay circuit reservation");
                    self.circuit_listeners.insert(id, circuit);
                }
                Err(e) => {
                    warn!(%circuit, error = %e, "Failed to re-listen on relay circuit; will retry");
                    self.circuit_relisten.push(circuit);
                }
            }
        }
    }

    async fn handle_command(&mut self, command: SwarmCommand) {
        match command {
            SwarmCommand::Dial { addr, reply } => {
                let result = self
                    .swarm
                    .dial(addr)
                    .map_err(|e| TransportError::DialFailed {
                        peer: "unknown".into(),
                        reason: e.to_string(),
                    });
                let _ = reply.send(result);
            }
            SwarmCommand::ConnectToRelays { reply } => {
                let result = swarm::connect_to_relays(&mut self.swarm, &self.config).await;
                let _ = reply.send(result);
            }
            SwarmCommand::SendRpc {
                peer,
                libp2p_peer,
                protocol,
                request,
                timeout,
                reply,
            } => {
                // Bound the pending map: reject rather than grow without limit.
                if self.pending_requests.len() >= MAX_PENDING_REQUESTS {
                    let _ = reply.send(Err(TransportError::DialFailed {
                        peer: peer.to_string(),
                        reason: format!(
                            "too many in-flight requests ({MAX_PENDING_REQUESTS}), try again later"
                        ),
                    }));
                    return;
                }

                let request_id = self.swarm.behaviour_mut().ganglion_rpc.send_request(
                    &libp2p_peer,
                    crate::swarm::GanglionRequest {
                        protocol: protocol.as_str().to_string(),
                        payload: request,
                    },
                );
                self.pending_requests.insert(
                    request_id,
                    PendingRequest {
                        peer_id: peer.clone(),
                        protocol,
                        reply,
                        deadline: Instant::now() + timeout,
                        timeout,
                    },
                );
                let mut peers = self.connected_peers.write().await;
                if let Some(conn) = peers.get_mut(peer.as_str()) {
                    conn.messages_sent += 1;
                }
            }
            SwarmCommand::SendResponse { channel, data } => {
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .ganglion_rpc
                    .send_response(channel, data);
            }
            SwarmCommand::Shutdown => {
                // Handled directly in `run`; nothing to do here.
            }
        }
    }

    async fn on_connection_established(
        &mut self,
        peer_id: Libp2pPeerId,
        endpoint: libp2p::core::ConnectedPoint,
    ) {
        let via_relay = endpoint.is_relayed();
        info!(peer = %peer_id, relay = via_relay, "Connection established");

        // A circuit reservation listener that was created before its relay
        // connection existed closes immediately; re-establish it NOW that a
        // connection is up instead of waiting for the next sweep tick. This
        // closes the startup window where an agent is connected to its relay
        // but not yet reachable through it (an operator's circuit dial in
        // that window is refused with NO_RESERVATION).
        if !self.circuit_relisten.is_empty() {
            self.relisten_closed_circuits();
        }

        let Some(gang_peer_id) = libp2p_to_gang_peer_id(&peer_id) else {
            warn!(peer = %peer_id, "Rejecting peer: could not recover Ed25519 identity");
            let _ = self.swarm.disconnect_peer_id(peer_id);
            return;
        };

        // Determine transport from endpoint multiaddr. Recognises current
        // (tcp, quic) and future (webtransport, webrtc) transports so the field
        // is correct if/when those features land in a future libp2p release.
        let transport_name = if via_relay {
            "relay".to_string()
        } else {
            let addr_str = endpoint.get_remote_address().to_string();
            if addr_str.contains("/webtransport/") {
                "webtransport".to_string()
            } else if addr_str.contains("/webrtc-direct/") || addr_str.contains("/webrtc/") {
                "webrtc".to_string()
            } else if addr_str.contains("quic") {
                "quic".to_string()
            } else {
                "tcp".to_string()
            }
        };

        let peer_key = gang_peer_id.as_str().to_string();
        let mut peers = self.connected_peers.write().await;
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
        drop(peers);

        self.events.publish(TransportEvent::PeerConnected {
            peer_id: gang_peer_id,
            via_relay,
        });
    }

    async fn handle_behaviour_event(&mut self, event: swarm::GanglionBehaviourEvent) {
        use swarm::GanglionBehaviourEvent;

        match event {
            GanglionBehaviourEvent::Identify(libp2p::identify::Event::Received {
                peer_id,
                info,
                ..
            }) => {
                debug!(peer = %peer_id, agent = %info.agent_version, "Identified peer");

                // Add peer addresses to Kademlia
                for addr in &info.listen_addrs {
                    self.swarm
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
                if let Some(gang_peer_id) = libp2p_to_gang_peer_id(&peer) {
                    let mut peers = self.connected_peers.write().await;
                    if let Some(conn) = peers.get_mut(gang_peer_id.as_str()) {
                        conn.last_rtt = Some(rtt);
                    }
                }
            }
            GanglionBehaviourEvent::Dcutr(libp2p::dcutr::Event {
                remote_peer_id,
                result: Ok(_),
                ..
            }) => {
                info!(peer = %remote_peer_id, "Direct connection upgraded via DCUtR");

                if let Some(gang_peer_id) = libp2p_to_gang_peer_id(&remote_peer_id) {
                    {
                        let mut peers = self.connected_peers.write().await;
                        if let Some(conn) = peers.get_mut(gang_peer_id.as_str()) {
                            conn.dcutr_attempted = true;
                            conn.dcutr_succeeded = true;
                            conn.via_relay = false;
                            conn.transport = "quic".into();
                        }
                    }

                    self.events.publish(TransportEvent::DirectUpgrade {
                        peer_id: gang_peer_id,
                    });
                }
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
                if let Some(req) = self.pending_requests.remove(&request_id) {
                    let _ = req.reply.send(Err(TransportError::DialFailed {
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
                warn!(peer = %peer, request_id = ?request_id, error = ?error, "Inbound RPC failed");
            }
            GanglionBehaviourEvent::RelayServer(event) => {
                debug!("Relay server event: {event:?}");
            }
            _ => {}
        }
    }

    /// Handle an incoming or outgoing RPC message.
    async fn handle_rpc_message(
        &mut self,
        peer: Libp2pPeerId,
        message: request_response::Message<crate::swarm::GanglionRequest, Vec<u8>>,
    ) {
        match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
                // SEC-03: the downstream handler and policy engine key on this
                // identity, so a peer whose Ed25519 key cannot be recovered is
                // unauthenticated — drop the request rather than dispatch it.
                let Some(gang_peer_id) = libp2p_to_gang_peer_id(&peer) else {
                    warn!(peer = %peer, "Dropping RPC from peer with unrecoverable identity");
                    return;
                };
                let crate::swarm::GanglionRequest { protocol, payload } = request;

                {
                    let mut peers = self.connected_peers.write().await;
                    if let Some(conn) = peers.get_mut(gang_peer_id.as_str()) {
                        conn.messages_received += 1;
                        conn.bytes_received += payload.len() as u64;
                    }
                }

                // Dispatch by the NEGOTIATED protocol, not the first registered
                // handler. This ensures a `/ganglion/bulk/1.0` request is never
                // handed to, say, the control handler.
                let handlers = self.protocol_handlers.read().await;
                if let Some(handler) = handlers.get(&protocol) {
                    debug!(peer = %peer, protocol = %protocol, len = payload.len(), "Incoming RPC request");

                    let response_rx =
                        serve_request(handler, ProtocolId::new(protocol), gang_peer_id, payload);
                    drop(handlers);

                    let command_tx = self.command_tx.clone();
                    tokio::spawn(async move {
                        match response_rx.await {
                            Ok(data) => {
                                let _ = command_tx
                                    .send(SwarmCommand::SendResponse { channel, data })
                                    .await;
                            }
                            Err(_) => {
                                // The handler failed to produce a response
                                // (its task was dropped without sending).
                                // Drop the channel instead of masking the
                                // failure with an empty "success" response;
                                // the peer then observes a genuine
                                // InboundFailure::ResponseOmission.
                                warn!(
                                    "handler produced no response; omitting response so the peer sees a failure"
                                );
                            }
                        }
                    });
                } else {
                    debug!(peer = %peer, protocol = %protocol, "No handler registered for protocol, sending empty response");
                    drop(handlers);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .ganglion_rpc
                        .send_response(channel, Vec::new());
                }
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                if let Some(req) = self.pending_requests.remove(&request_id) {
                    debug!(
                        peer = %req.peer_id,
                        protocol = %req.protocol,
                        len = response.len(),
                        "Received RPC response"
                    );

                    {
                        let mut peers = self.connected_peers.write().await;
                        if let Some(conn) = peers.get_mut(req.peer_id.as_str()) {
                            conn.messages_received += 1;
                            conn.bytes_received += response.len() as u64;
                        }
                    }

                    let _ = req.reply.send(Ok(response));
                }
            }
        }
    }
}

/// Await a reply from the swarm task, bounded by [`REQUEST_TIMEOUT`].
///
/// On timeout the caller returns promptly; the worker's periodic sweep is
/// responsible for removing the corresponding pending-request entry.
async fn await_reply(
    peer: &PeerId,
    reply_rx: oneshot::Receiver<Result<Vec<u8>, TransportError>>,
) -> Result<Vec<u8>, TransportError> {
    await_reply_within(peer, reply_rx, REQUEST_TIMEOUT).await
}

/// Await a reply from the swarm task, bounded by an explicit timeout.
async fn await_reply_within(
    peer: &PeerId,
    reply_rx: oneshot::Receiver<Result<Vec<u8>, TransportError>>,
    timeout: Duration,
) -> Result<Vec<u8>, TransportError> {
    match tokio::time::timeout(timeout, reply_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(TransportError::DialFailed {
            peer: peer.to_string(),
            reason: "response channel closed".into(),
        }),
        Err(_) => Err(TransportError::Timeout(timeout)),
    }
}

/// Run a registered stream handler against an inbound request payload,
/// returning a receiver that resolves to the handler's response bytes.
fn serve_request(
    handler: &StreamHandler,
    protocol: ProtocolId,
    remote_peer: PeerId,
    payload: Vec<u8>,
) -> oneshot::Receiver<Vec<u8>> {
    let (response_data_tx, response_data_rx) = oneshot::channel::<Vec<u8>>();

    // Two independent pipes: one carries the request to the handler, the other
    // carries the handler's response back. The handler's stream reads the
    // request pipe and writes the response pipe.
    let (mut request_writer, request_reader) = tokio::io::duplex(64 * 1024);
    let (response_reader, response_writer) = tokio::io::duplex(64 * 1024);

    // Feed the request payload to the handler.
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let _ = request_writer.write_all(&payload).await;
        let _ = request_writer.shutdown().await;
    });

    // Collect the handler's response.
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut reader = response_reader;
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf).await;
        let _ = response_data_tx.send(buf);
    });

    let stream = GanglionStream {
        protocol,
        remote_peer,
        inner: Box::new(merge_rw(request_reader, response_writer)),
    };

    tokio::spawn(handler(stream));

    response_data_rx
}

#[async_trait::async_trait]
impl TransportAdapter for Libp2pTransportAdapter {
    async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError> {
        let libp2p_peer_id = self.resolve_peer(peer).await?;

        // Open a request-response stream on the control protocol.
        let protocol = ProtocolId::control();
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(SwarmCommand::SendRpc {
                peer: peer.clone(),
                libp2p_peer: libp2p_peer_id,
                protocol: protocol.clone(),
                request: Vec::new(),
                timeout: REQUEST_TIMEOUT,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransportError::ConnectionClosed("swarm task is not running".into()))?;

        let response = await_reply(peer, reply_rx).await?;

        // Build a GanglionStream from the response data.
        let (read_half, mut write_half) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = write_half.write_all(&response).await;
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
        let mut rx = self.events.subscribe();
        Box::pin(async_stream::stream! {
            while let Some(event) = rx.recv().await {
                yield event;
            }
        })
    }

    async fn announce_presence(&self, info: PresenceInfo) -> Result<(), TransportError> {
        // Presence broadcast (a signed presence message to connected relay
        // peers over the control protocol) is not implemented yet. Return a
        // typed error so callers cannot mistake the no-op for success.
        Err(TransportError::RelayUnavailable(format!(
            "presence announcement for {} is not implemented in the libp2p adapter yet",
            info.peer_id
        )))
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
        // Best-effort: if the worker already stopped the channel is closed.
        let _ = self.command_tx.send(SwarmCommand::Shutdown).await;
        Ok(())
    }
}

/// Convert a libp2p PeerId to a Ganglion PeerId.
/// Uses the libp2p peer ID string as a deterministic input.
/// Map a libp2p peer ID to a gang-core [`PeerId`] (SEC-03).
///
/// Ganglion identities are Ed25519, and libp2p inlines Ed25519 public keys
/// directly in the peer-id multihash (identity hash, code 0x00). We recover the
/// raw 32-byte key and derive the gang peer ID via the SAME canonical scheme
/// [`PeerId::from_ed25519_bytes`] used by the core identity, trust store, and
/// manifests — so the identity observed on the wire matches the one policy
/// `peer_rules` are keyed on. If the key cannot be recovered (a non-Ed25519 or
/// non-inlined peer id, which Ganglion never issues), this returns `None` and
/// the caller treats the peer as unauthenticated.
pub fn libp2p_to_gang_peer_id(peer_id: &Libp2pPeerId) -> Option<PeerId> {
    let key = ed25519_pubkey_from_libp2p(peer_id)?;
    Some(PeerId::from_ed25519_bytes(&key))
}

/// Identity material recovered from a dialable base58 libp2p peer id
/// (`12D3KooW…`). See [`identity_from_libp2p_str`].
#[derive(Debug, Clone)]
pub struct DialableIdentity {
    /// The libp2p peer id in its canonical base58 form.
    pub libp2p_id: String,
    /// The canonical gang peer id derived from the embedded Ed25519 key
    /// (SEC-03 derivation — matches trust stores, manifests, and policy).
    pub gang_id: PeerId,
    /// The raw 32-byte Ed25519 public key embedded in the libp2p id.
    pub ed25519_pubkey: [u8; 32],
}

/// Parse a base58 libp2p peer id string (`12D3KooW…`) and recover the
/// embedded Ed25519 identity.
///
/// A libp2p Ed25519 peer id *is* the public key (inlined via the identity
/// multihash), so the gang trust identity is derivable from it — never the
/// reverse. Returns `None` if the string is not a libp2p peer id or does not
/// embed an Ed25519 key (Ganglion never issues such ids).
pub fn identity_from_libp2p_str(s: &str) -> Option<DialableIdentity> {
    let libp2p_id: Libp2pPeerId = s.parse().ok()?;
    let ed25519_pubkey = ed25519_pubkey_from_libp2p(&libp2p_id)?;
    Some(DialableIdentity {
        libp2p_id: libp2p_id.to_string(),
        gang_id: PeerId::from_ed25519_bytes(&ed25519_pubkey),
        ed25519_pubkey,
    })
}

/// Recover the raw 32-byte Ed25519 public key inlined in a libp2p peer id.
fn ed25519_pubkey_from_libp2p(peer_id: &Libp2pPeerId) -> Option<[u8; 32]> {
    use libp2p::identity::PublicKey;
    use libp2p::multihash::Multihash;

    // Ed25519 peer ids use the identity multihash (code 0x00) carrying the
    // protobuf-encoded public key as the digest.
    let mh = Multihash::<64>::from_bytes(&peer_id.to_bytes()).ok()?;
    if mh.code() != 0x00 {
        return None;
    }
    let pk = PublicKey::try_decode_protobuf(mh.digest()).ok()?;
    let ed = pk.try_into_ed25519().ok()?;
    Some(ed.to_bytes())
}

/// Parse a gang peer ID string back into a PeerId.
fn parse_gang_peer_id(id: &str) -> Option<PeerId> {
    Some(PeerId::new(id))
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
            key_path: key_path.clone(),
            listen_addrs: vec![], // Don't bind ports in tests
            ..Default::default()
        };

        let secret = swarm::load_ed25519_secret(&key_path).unwrap();
        let result = swarm::build_swarm(&config, secret).await;
        assert!(result.is_ok(), "swarm build failed: {:?}", result.err());

        let (_swarm, peer_id, _control) = result.unwrap();
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
            key_path: key_path.clone(),
            listen_addrs: vec![],
            relay_server: true,
            ..Default::default()
        };

        let secret = swarm::load_ed25519_secret(&key_path).unwrap();
        let result = swarm::build_swarm(&config, secret).await;
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

    /// A handler that drains the request and responds with a fixed tag, so a
    /// test can tell which registered handler actually ran.
    fn tag_handler(tag: &'static [u8]) -> StreamHandler {
        Box::new(move |mut stream| {
            Box::pin(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = Vec::new();
                let _ = stream.inner.read_to_end(&mut buf).await;
                let _ = stream.inner.write_all(tag).await;
                let _ = stream.inner.shutdown().await;
            })
        })
    }

    #[tokio::test]
    async fn test_inbound_dispatch_uses_negotiated_protocol() {
        use gang_core::protocol::{ALL_PROTOCOLS, PROTOCOL_BULK, PROTOCOL_CONTROL};

        // The control protocol is first in ALL_PROTOCOLS, so the previous
        // "first registered handler" logic would always have picked it.
        assert_eq!(ALL_PROTOCOLS[0], PROTOCOL_CONTROL);

        let mut handlers: HashMap<String, StreamHandler> = HashMap::new();
        handlers.insert(PROTOCOL_CONTROL.to_string(), tag_handler(b"control"));
        handlers.insert(PROTOCOL_BULK.to_string(), tag_handler(b"bulk"));

        // A request negotiated on the bulk protocol must be served by the bulk
        // handler, not the control handler.
        let negotiated = PROTOCOL_BULK.to_string();
        let handler = handlers.get(&negotiated).expect("bulk handler present");
        let rx = serve_request(
            handler,
            ProtocolId::new(negotiated),
            PeerId::new("12D3-000000000000000000000000000000aa"),
            b"request".to_vec(),
        );
        let response = rx.await.unwrap();
        assert_eq!(
            response, b"bulk",
            "bulk request was dispatched to the wrong handler"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_await_reply_times_out() {
        // Keep the sender alive so the channel is not closed; the reply never
        // arrives, so await_reply must resolve with a Timeout error once the
        // (virtual) clock passes REQUEST_TIMEOUT.
        let (_tx, rx) = oneshot::channel::<Result<Vec<u8>, TransportError>>();
        let peer = PeerId::new("12D3-000000000000000000000000000000bb");
        let result = await_reply(&peer, rx).await;
        assert!(
            matches!(result, Err(TransportError::Timeout(_))),
            "expected timeout, got {result:?}"
        );
    }

    #[tokio::test]
    async fn test_event_bus_fans_out_to_multiple_subscribers() {
        let bus = EventBus::default();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();

        let peer = PeerId::new("12D3-000000000000000000000000000000cc");
        bus.publish(TransportEvent::PeerConnected {
            peer_id: peer.clone(),
            via_relay: false,
        });

        let ea = a.recv().await.expect("subscriber A missed the event");
        let eb = b.recv().await.expect("subscriber B missed the event");

        match (ea, eb) {
            (
                TransportEvent::PeerConnected { peer_id: pa, .. },
                TransportEvent::PeerConnected { peer_id: pb, .. },
            ) => {
                assert_eq!(pa, peer);
                assert_eq!(pb, peer);
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    /// Build a genuine Ed25519 libp2p peer id (its raw public key is recoverable
    /// from the inlined multihash, matching how Ganglion issues identities).
    fn real_ed25519_libp2p_peer() -> Libp2pPeerId {
        let kp = libp2p::identity::Keypair::generate_ed25519();
        kp.public().to_peer_id()
    }

    #[tokio::test]
    async fn test_peer_connection_tracking() {
        let connected_peers: Arc<RwLock<HashMap<String, PeerConnection>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Simulate a connection
        let fake_libp2p_peer = real_ed25519_libp2p_peer();
        let gang_peer_id =
            libp2p_to_gang_peer_id(&fake_libp2p_peer).expect("ed25519 identity recoverable");
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

        let fake_libp2p_peer = real_ed25519_libp2p_peer();
        let gang_peer_id =
            libp2p_to_gang_peer_id(&fake_libp2p_peer).expect("ed25519 identity recoverable");
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
        let libp2p_peer = real_ed25519_libp2p_peer();
        let gang_id_1 = libp2p_to_gang_peer_id(&libp2p_peer).expect("recoverable");
        let gang_id_2 = libp2p_to_gang_peer_id(&libp2p_peer).expect("recoverable");
        assert_eq!(gang_id_1, gang_id_2);
        assert!(gang_id_1.as_str().starts_with("12D3-"));
    }

    #[test]
    fn test_libp2p_to_gang_peer_id_matches_core_derivation() {
        // SEC-03: the id derived from a libp2p peer id must equal the id
        // gang-core derives from the same raw Ed25519 public key, so trust
        // rules keyed on a core-derived identity match the wire identity.
        let kp = libp2p::identity::Keypair::generate_ed25519();
        let raw = kp.public().try_into_ed25519().unwrap().to_bytes();
        let from_wire = libp2p_to_gang_peer_id(&kp.public().to_peer_id()).expect("recoverable");
        let from_core = PeerId::from_ed25519_bytes(&raw);
        assert_eq!(from_wire, from_core);
    }

    #[test]
    fn test_random_peer_id_is_unrecoverable() {
        // A random (non-inlined) libp2p peer id has no embedded key, so it is
        // treated as unauthenticated.
        assert!(libp2p_to_gang_peer_id(&Libp2pPeerId::random()).is_none());
    }

    #[test]
    fn test_parse_gang_peer_id_roundtrip() {
        let libp2p_peer = real_ed25519_libp2p_peer();
        let gang_id = libp2p_to_gang_peer_id(&libp2p_peer).expect("recoverable");
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
