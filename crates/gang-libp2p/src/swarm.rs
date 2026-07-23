use std::time::Duration;

use libp2p::{
    Multiaddr, PeerId as Libp2pPeerId, Swarm, SwarmBuilder, connection_limits, dcutr, identify,
    kad, noise, ping, relay, request_response,
    swarm::{NetworkBehaviour, behaviour::toggle::Toggle},
    tcp, yamux,
};
use tracing::{debug, info};

use crate::config::Libp2pConfig;

/// Combined network behaviour for a Ganglion node.
#[derive(NetworkBehaviour)]
pub struct GanglionBehaviour {
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub relay_client: relay::client::Behaviour,
    pub dcutr: dcutr::Behaviour,
    /// Circuit-relay v2 server. Disabled (no-op) unless `config.relay_server`
    /// is set, so client robots never relay traffic for arbitrary peers.
    pub relay_server: Toggle<relay::Behaviour>,
    /// Enforces `config.max_inbound_connections` on established inbound links.
    pub connection_limits: connection_limits::Behaviour,
    pub ganglion_rpc: request_response::Behaviour<GanglionCodec>,
}

/// Maximum message size for Ganglion request/response (16 MiB).
const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Simple length-prefixed codec for Ganglion stream protocols.
/// Sends and receives raw bytes with a 4-byte big-endian length prefix.
#[derive(Debug, Clone, Default)]
pub struct GanglionCodec;

/// Protocol name type wrapping a static string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GanglionProtocol(pub String);

impl AsRef<str> for GanglionProtocol {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// An inbound Ganglion request, tagged with the protocol the connection
/// actually negotiated.
///
/// Carrying the negotiated protocol id lets the adapter dispatch each request
/// to the handler registered for that exact protocol, rather than guessing the
/// first registered handler (which could feed a `/ganglion/bulk/1.0` payload
/// to the control handler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GanglionRequest {
    /// The protocol id negotiated for this request (e.g. `/ganglion/bulk/1.0`).
    pub protocol: String,
    /// The raw request payload.
    pub payload: Vec<u8>,
}

#[async_trait::async_trait]
impl request_response::Codec for GanglionCodec {
    type Protocol = GanglionProtocol;
    type Request = GanglionRequest;
    type Response = Vec<u8>;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Request>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        let payload = read_length_prefixed(io).await?;
        Ok(GanglionRequest {
            protocol: protocol.as_ref().to_string(),
            payload,
        })
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Response>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        read_length_prefixed(io).await
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        // Only the payload goes on the wire; the protocol is negotiated by
        // multistream-select, not carried in the message body.
        write_length_prefixed(io, &req.payload).await
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, &res).await
    }
}

/// Initial read buffer capacity. The buffer grows as data actually arrives
/// (bounded by the declared length), so a peer cannot cheaply pin large
/// allocations by sending only a length prefix and then stalling.
const INITIAL_READ_CAPACITY: usize = 8 * 1024;

/// Read a length-prefixed message from a stream.
///
/// The declared length is validated against [`MAX_MESSAGE_SIZE`], but the
/// buffer is NOT pre-allocated to that length. Instead we cap the reader at
/// the declared length and read incrementally into a buffer that grows with
/// received data. An idle stream that announces a huge length but never sends
/// the body only ever holds the small initial allocation, not up to 16 MiB.
async fn read_length_prefixed<T>(io: &mut T) -> std::io::Result<Vec<u8>>
where
    T: futures::AsyncRead + Unpin + Send,
{
    use futures::AsyncReadExt;

    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("message too large: {len} bytes (max {MAX_MESSAGE_SIZE})"),
        ));
    }

    // Grow-on-demand buffer capped at the declared length.
    let mut buf = Vec::with_capacity(len.min(INITIAL_READ_CAPACITY));
    let read = io.take(len as u64).read_to_end(&mut buf).await?;
    if read != len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("expected {len} bytes, got {read} before EOF"),
        ));
    }
    Ok(buf)
}

/// Write a length-prefixed message to a stream.
async fn write_length_prefixed<T>(io: &mut T, data: &[u8]) -> std::io::Result<()>
where
    T: futures::AsyncWrite + Unpin + Send,
{
    use futures::AsyncWriteExt;

    let len = data.len() as u32;
    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(data).await?;
    io.close().await?;
    Ok(())
}

/// Load the raw 32-byte Ed25519 secret key from `path`.
///
/// Reads the file exactly once and validates the length before copying,
/// so a truncated or oversized key file returns an error instead of
/// panicking (the previous `copy_from_slice` on an unchecked slice could
/// panic). Callers pass the returned bytes into [`build_swarm`], which
/// avoids re-reading the file (removing a TOCTOU second read).
pub(crate) fn load_ed25519_secret(path: &std::path::Path) -> anyhow::Result<[u8; 32]> {
    let bytes = std::fs::read(path)?;
    if bytes.len() != 32 {
        anyhow::bail!(
            "expected 32-byte ed25519 key at {}, got {} bytes",
            path.display(),
            bytes.len()
        );
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Build and configure the libp2p swarm from a Ganglion config.
///
/// `secret_key` is the already-loaded raw Ed25519 secret (see
/// [`load_ed25519_secret`]); this function does not read the key file, so
/// there is no second on-disk read that could race with the initial load.
pub async fn build_swarm(
    config: &Libp2pConfig,
    secret_key: [u8; 32],
) -> anyhow::Result<(Swarm<GanglionBehaviour>, Libp2pPeerId)> {
    // Convert the raw secret into a libp2p keypair (both are Ed25519).
    // `ed25519_from_bytes` takes the bytes by value and zeroizes them.
    let libp2p_keypair = libp2p::identity::Keypair::ed25519_from_bytes(secret_key)?;
    let local_peer_id = libp2p_keypair.public().to_peer_id();

    info!(%local_peer_id, relay_server = config.relay_server, "Building Ganglion swarm");

    let swarm = SwarmBuilder::with_existing_identity(libp2p_keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|keypair, relay_client| {
            let local_peer_id = keypair.public().to_peer_id();

            // Identify: lets peers exchange metadata
            let identify = identify::Behaviour::new(
                identify::Config::new("/ganglion/0.1.0".into(), keypair.public())
                    .with_push_listen_addr_updates(true),
            );

            // Ping: keepalive and latency measurement
            let ping =
                ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(30)));

            // Kademlia: peer routing through bootstrap nodes
            let mut kademlia =
                kad::Behaviour::new(local_peer_id, kad::store::MemoryStore::new(local_peer_id));
            kademlia.set_mode(Some(kad::Mode::Client));

            // DCUtR: direct connection upgrade through relay
            let dcutr = dcutr::Behaviour::new(local_peer_id);

            // Relay server: circuit relay v2 for other peers. Only enabled when
            // this node is explicitly configured as a relay; otherwise a
            // disabled Toggle is installed so we never relay for arbitrary peers.
            let relay_server: Toggle<relay::Behaviour> = if config.relay_server {
                Toggle::from(Some(relay::Behaviour::new(
                    local_peer_id,
                    relay::Config::default(),
                )))
            } else {
                Toggle::from(None)
            };

            // Enforce the configured inbound connection ceiling.
            let connection_limits = connection_limits::Behaviour::new(
                connection_limits::ConnectionLimits::default()
                    .with_max_established_incoming(Some(config.max_inbound_connections)),
            );

            // Ganglion request/response protocols
            let ganglion_rpc = request_response::Behaviour::with_codec(
                GanglionCodec,
                ganglion_protocols(),
                request_response::Config::default(),
            );

            Ok(GanglionBehaviour {
                identify,
                ping,
                kademlia,
                relay_client,
                dcutr,
                relay_server,
                connection_limits,
                ganglion_rpc,
            })
        })?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(Duration::from_secs(config.idle_timeout_secs))
        })
        .build();

    Ok((swarm, local_peer_id))
}

/// Return the set of Ganglion stream protocols with full inbound+outbound support.
fn ganglion_protocols() -> Vec<(GanglionProtocol, request_response::ProtocolSupport)> {
    use gang_core::protocol::ALL_PROTOCOLS;

    ALL_PROTOCOLS
        .iter()
        .map(|p| {
            (
                GanglionProtocol(p.to_string()),
                request_response::ProtocolSupport::Full,
            )
        })
        .collect()
}

/// Start listening on configured addresses.
pub fn start_listening(
    swarm: &mut Swarm<GanglionBehaviour>,
    config: &Libp2pConfig,
) -> anyhow::Result<()> {
    for addr_str in &config.listen_addrs {
        let addr: Multiaddr = addr_str.parse()?;
        swarm.listen_on(addr)?;
        info!("Listening on {addr_str}");
    }
    Ok(())
}

/// Connect to configured relay nodes.
pub async fn connect_to_relays(
    swarm: &mut Swarm<GanglionBehaviour>,
    config: &Libp2pConfig,
) -> anyhow::Result<()> {
    for addr_str in &config.relay_addrs {
        let addr: Multiaddr = addr_str.parse()?;
        info!("Dialing relay at {addr_str}");
        swarm.dial(addr)?;
    }
    Ok(())
}

/// Add bootstrap peers to Kademlia.
pub fn add_bootstrap_peers(
    swarm: &mut Swarm<GanglionBehaviour>,
    config: &Libp2pConfig,
) -> anyhow::Result<()> {
    for addr_str in &config.bootstrap_peers {
        let addr: Multiaddr = addr_str.parse()?;
        if let Some(peer_id) = extract_peer_id(&addr) {
            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
            debug!("Added bootstrap peer {peer_id}");
        }
    }
    Ok(())
}

/// Extract a PeerId from a Multiaddr (if it contains a /p2p/ component).
fn extract_peer_id(addr: &Multiaddr) -> Option<Libp2pPeerId> {
    addr.iter().find_map(|p| match p {
        libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn build_test_swarm(relay_server: bool, max_inbound: u32) -> Swarm<GanglionBehaviour> {
        let tmpdir = tempfile::tempdir().unwrap();
        let key_path = tmpdir.path().join("test.key");
        gang_core::identity::Keypair::generate()
            .save(&key_path)
            .unwrap();
        let config = Libp2pConfig {
            key_path: key_path.clone(),
            listen_addrs: vec![],
            relay_server,
            max_inbound_connections: max_inbound,
            ..Default::default()
        };
        let secret = load_ed25519_secret(&key_path).unwrap();
        build_swarm(&config, secret).await.unwrap().0
    }

    #[tokio::test]
    async fn test_relay_server_disabled_by_default() {
        let swarm = build_test_swarm(false, 64).await;
        assert!(
            !swarm.behaviour().relay_server.is_enabled(),
            "relay server must be disabled unless config.relay_server is set"
        );
    }

    #[tokio::test]
    async fn test_relay_server_enabled_when_configured() {
        let swarm = build_test_swarm(true, 64).await;
        assert!(
            swarm.behaviour().relay_server.is_enabled(),
            "relay server should be enabled when config.relay_server is true"
        );
    }

    #[tokio::test]
    async fn test_connection_limits_present() {
        // Building with a custom inbound ceiling must succeed and wire in the
        // connection-limits behaviour (the field is non-optional).
        let swarm = build_test_swarm(false, 8).await;
        let _limits = &swarm.behaviour().connection_limits;
    }

    #[tokio::test]
    async fn test_codec_read_request_tags_negotiated_protocol() {
        use request_response::Codec;

        // Encode a payload on the wire (length-prefixed, no protocol on wire).
        let payload = b"bulk payload".to_vec();
        let mut wire = Vec::new();
        write_length_prefixed(&mut wire, &payload).await.unwrap();

        // Read it back as if negotiated on the bulk protocol.
        let mut codec = GanglionCodec;
        let proto = GanglionProtocol(gang_core::protocol::PROTOCOL_BULK.to_string());
        let mut cursor = futures::io::Cursor::new(wire);
        let req = codec.read_request(&proto, &mut cursor).await.unwrap();

        assert_eq!(req.protocol, gang_core::protocol::PROTOCOL_BULK);
        assert_eq!(req.payload, payload);
    }

    #[tokio::test]
    async fn test_length_prefixed_roundtrip() {
        let payload = b"hello ganglion".to_vec();
        let mut wire = Vec::new();
        write_length_prefixed(&mut wire, &payload).await.unwrap();

        let mut cursor = futures::io::Cursor::new(wire);
        let read = read_length_prefixed(&mut cursor).await.unwrap();
        assert_eq!(read, payload);
    }

    #[tokio::test]
    async fn test_length_prefixed_rejects_oversized() {
        // Length prefix declaring more than MAX_MESSAGE_SIZE, no body.
        let mut wire = Vec::new();
        wire.extend_from_slice(&((MAX_MESSAGE_SIZE as u32) + 1).to_be_bytes());

        let mut cursor = futures::io::Cursor::new(wire);
        let err = read_length_prefixed(&mut cursor).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn test_length_prefixed_short_body_errors_without_preallocating() {
        // Declares a large body but only sends the prefix then EOFs. The read
        // must fail with UnexpectedEof rather than pinning a 4 MiB buffer.
        let declared = 4 * 1024 * 1024u32;
        let mut wire = Vec::new();
        wire.extend_from_slice(&declared.to_be_bytes());
        wire.extend_from_slice(b"only a few bytes");

        let mut cursor = futures::io::Cursor::new(wire);
        let err = read_length_prefixed(&mut cursor).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
