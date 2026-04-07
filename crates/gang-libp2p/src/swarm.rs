use std::time::Duration;

use libp2p::{
    Multiaddr, PeerId as Libp2pPeerId, Swarm, SwarmBuilder, dcutr, identify, kad, noise, ping,
    relay, request_response, swarm::NetworkBehaviour, tcp, yamux,
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
    pub relay_server: relay::Behaviour,
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

#[async_trait::async_trait]
impl request_response::Codec for GanglionCodec {
    type Protocol = GanglionProtocol;
    type Request = Vec<u8>;
    type Response = Vec<u8>;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Request>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        read_length_prefixed(io).await
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
        write_length_prefixed(io, &req).await
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

/// Read a length-prefixed message from a stream.
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

    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;
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

/// Build and configure the libp2p swarm from a Ganglion config.
pub async fn build_swarm(
    config: &Libp2pConfig,
) -> anyhow::Result<(Swarm<GanglionBehaviour>, Libp2pPeerId)> {
    // Load or generate identity
    let _gang_keypair = gang_core::identity::Keypair::load_or_generate(&config.key_path)?;

    // Convert to libp2p keypair (both are Ed25519)
    let key_bytes = std::fs::read(&config.key_path)?;
    let mut ed25519_bytes = [0u8; 32];
    ed25519_bytes.copy_from_slice(&key_bytes);

    let libp2p_keypair = libp2p::identity::Keypair::ed25519_from_bytes(ed25519_bytes)?;
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

            // Relay server: circuit relay v2 for other peers
            let relay_server = relay::Behaviour::new(local_peer_id, relay::Config::default());

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
