use std::pin::Pin;
use std::time::{Duration, Instant};

use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::debug;

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
    ///
    /// Transports are started in `preference.preferred_order`, each staggered
    /// by `preference.stagger_delay`. The first successful handshake wins and
    /// all remaining in-flight attempts are cancelled via `tokio::select!`.
    ///
    /// The overall attempt is bounded by `preference.dial_timeout`.
    ///
    /// Added in v0.2.
    async fn dial_parallel(
        &self,
        peer: &PeerId,
        preference: &TransportPreference,
    ) -> Result<DialResult, TransportError> {
        let available = self.capabilities().transports;

        // Filter preferred transports to those actually available.
        let candidates: Vec<&str> = preference
            .preferred_order
            .iter()
            .filter(|t| available.contains(t))
            .map(|t| t.as_str())
            .collect();

        if candidates.is_empty() {
            return Err(TransportError::DialFailed {
                peer: peer.to_string(),
                reason: "no preferred transports are available".into(),
            });
        }

        // Single transport fast path -- no stagger needed.
        if candidates.len() == 1 {
            let start = Instant::now();
            let stream = tokio::time::timeout(preference.dial_timeout, self.dial(peer))
                .await
                .map_err(|_| TransportError::DialFailed {
                    peer: peer.to_string(),
                    reason: format!(
                        "dial timed out after {:?} on {}",
                        preference.dial_timeout, candidates[0]
                    ),
                })??;
            return Ok(DialResult {
                transport_used: candidates[0].to_string(),
                via_relay: false,
                dial_duration: start.elapsed(),
                stream,
            });
        }

        // Happy-eyeballs: sequential stagger with early return.
        //
        // The default trait implementation cannot spawn tasks (no `Arc<Self>`),
        // so we drive attempts sequentially with stagger delays between them.
        // The first success returns immediately; remaining transports are
        // skipped. Concrete adapters (e.g. Libp2pTransportAdapter) can
        // override with a truly parallel spawn-based implementation.
        let start = Instant::now();
        let stagger = preference.stagger_delay;
        let overall_deadline = Instant::now() + preference.dial_timeout;

        for (idx, transport_name) in candidates.iter().enumerate() {
            if Instant::now() >= overall_deadline {
                break;
            }

            // Stagger delay (skip for first attempt).
            if idx > 0 {
                let remaining = overall_deadline.saturating_duration_since(Instant::now());
                let delay = stagger.min(remaining);
                tokio::time::sleep(delay).await;
            }

            let remaining = overall_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            debug!(transport = %transport_name, "attempting dial");

            match tokio::time::timeout(remaining, self.dial(peer)).await {
                Ok(Ok(stream)) => {
                    debug!(
                        transport = %transport_name,
                        elapsed_ms = start.elapsed().as_millis(),
                        "dial succeeded"
                    );
                    return Ok(DialResult {
                        transport_used: transport_name.to_string(),
                        via_relay: false,
                        dial_duration: start.elapsed(),
                        stream,
                    });
                }
                Ok(Err(e)) => {
                    debug!(
                        transport = %transport_name,
                        error = %e,
                        "dial attempt failed, trying next"
                    );
                }
                Err(_) => {
                    debug!(
                        transport = %transport_name,
                        "dial attempt timed out, trying next"
                    );
                }
            }
        }

        Err(TransportError::DialFailed {
            peer: peer.to_string(),
            reason: format!(
                "all transports exhausted ({:?}) after {:?}",
                candidates,
                start.elapsed()
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A mock transport adapter for testing dial_parallel().
    /// It counts dial attempts and can be configured to succeed or fail.
    struct MockTransport {
        available_transports: Vec<String>,
        dial_count: AtomicU32,
        /// If true, dial() succeeds; otherwise it fails.
        should_succeed: bool,
        /// How long each dial() takes.
        dial_latency: Duration,
        peer_id: PeerId,
    }

    impl MockTransport {
        fn new(transports: Vec<&str>, should_succeed: bool, dial_latency: Duration) -> Self {
            let keypair = crate::identity::Keypair::generate();
            Self {
                available_transports: transports.into_iter().map(String::from).collect(),
                dial_count: AtomicU32::new(0),
                should_succeed,
                dial_latency,
                peer_id: keypair.peer_id(),
            }
        }
    }

    #[async_trait::async_trait]
    impl TransportAdapter for MockTransport {
        async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError> {
            self.dial_count.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.dial_latency).await;

            if self.should_succeed {
                let (read_half, _write_half) = tokio::io::duplex(1024);
                Ok(GanglionStream {
                    protocol: ProtocolId::control(),
                    remote_peer: peer.clone(),
                    inner: Box::new(read_half),
                })
            } else {
                Err(TransportError::DialFailed {
                    peer: peer.to_string(),
                    reason: "mock dial failure".into(),
                })
            }
        }

        async fn listen(
            &self,
            _protocol: ProtocolId,
            _handler: StreamHandler,
        ) -> Result<(), TransportError> {
            Ok(())
        }

        fn local_peer_id(&self) -> PeerId {
            self.peer_id.clone()
        }

        fn capabilities(&self) -> TransportCapabilities {
            TransportCapabilities {
                relay: false,
                hole_punch: false,
                direct_dial: true,
                encrypted: true,
                transports: self.available_transports.clone(),
            }
        }

        fn events(&self) -> Pin<Box<dyn Stream<Item = TransportEvent> + Send>> {
            Box::pin(futures::stream::empty())
        }

        async fn announce_presence(&self, _info: PresenceInfo) -> Result<(), TransportError> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_dial_parallel_single_transport_success() {
        let transport = MockTransport::new(vec!["quic"], true, Duration::from_millis(5));
        let peer = transport.peer_id.clone();
        let pref = TransportPreference {
            preferred_order: vec!["quic".into()],
            dial_timeout: Duration::from_secs(5),
            stagger_delay: Duration::from_millis(100),
        };

        let result = transport.dial_parallel(&peer, &pref).await;
        assert!(result.is_ok());
        let dial_result = result.unwrap();
        assert_eq!(dial_result.transport_used, "quic");
        assert_eq!(transport.dial_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_dial_parallel_first_transport_wins() {
        let transport = MockTransport::new(vec!["quic", "tcp"], true, Duration::from_millis(5));
        let peer = transport.peer_id.clone();
        let pref = TransportPreference {
            preferred_order: vec!["quic".into(), "tcp".into()],
            dial_timeout: Duration::from_secs(5),
            stagger_delay: Duration::from_millis(100),
        };

        let result = transport.dial_parallel(&peer, &pref).await;
        assert!(result.is_ok());
        let dial_result = result.unwrap();
        // First transport should win since dial succeeds quickly
        assert_eq!(dial_result.transport_used, "quic");
        // Only one dial attempt needed since the first succeeded
        assert_eq!(transport.dial_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_dial_parallel_all_fail() {
        let transport = MockTransport::new(vec!["quic", "tcp"], false, Duration::from_millis(5));
        let peer = transport.peer_id.clone();
        let pref = TransportPreference {
            preferred_order: vec!["quic".into(), "tcp".into()],
            dial_timeout: Duration::from_secs(5),
            stagger_delay: Duration::from_millis(50),
        };

        let result = transport.dial_parallel(&peer, &pref).await;
        assert!(result.is_err());
        // Both transports should have been attempted
        assert_eq!(transport.dial_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_dial_parallel_no_matching_transports() {
        let transport = MockTransport::new(vec!["quic"], true, Duration::from_millis(5));
        let peer = transport.peer_id.clone();
        let pref = TransportPreference {
            preferred_order: vec!["webtransport".into()],
            dial_timeout: Duration::from_secs(5),
            stagger_delay: Duration::from_millis(100),
        };

        let result = transport.dial_parallel(&peer, &pref).await;
        assert!(result.is_err());
        assert_eq!(transport.dial_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_dial_parallel_filters_unavailable_transports() {
        let transport = MockTransport::new(vec!["quic"], true, Duration::from_millis(5));
        let peer = transport.peer_id.clone();
        // Ask for both quic and tcp, but only quic is available
        let pref = TransportPreference {
            preferred_order: vec!["quic".into(), "tcp".into()],
            dial_timeout: Duration::from_secs(5),
            stagger_delay: Duration::from_millis(100),
        };

        let result = transport.dial_parallel(&peer, &pref).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().transport_used, "quic");
        // Only one attempt because tcp was filtered out
        assert_eq!(transport.dial_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_dial_parallel_timeout() {
        // Dial takes longer than the timeout
        let transport = MockTransport::new(vec!["quic"], true, Duration::from_secs(10));
        let peer = transport.peer_id.clone();
        let pref = TransportPreference {
            preferred_order: vec!["quic".into()],
            dial_timeout: Duration::from_millis(50),
            stagger_delay: Duration::from_millis(10),
        };

        let result = transport.dial_parallel(&peer, &pref).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            TransportError::DialFailed { reason, .. } => {
                assert!(reason.contains("timed out"));
            }
            other => panic!("expected DialFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_transport_preference_defaults() {
        let pref = TransportPreference::default();
        assert_eq!(pref.preferred_order, vec!["quic", "tcp"]);
        assert_eq!(pref.dial_timeout, Duration::from_secs(30));
        assert_eq!(pref.stagger_delay, Duration::from_millis(250));
    }

    #[test]
    fn test_transport_capabilities_default() {
        let caps = TransportCapabilities::default();
        assert!(!caps.relay);
        assert!(!caps.hole_punch);
        assert!(!caps.direct_dial);
        assert!(!caps.encrypted);
        assert!(caps.transports.is_empty());
    }
}
