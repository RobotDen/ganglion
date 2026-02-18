use std::time::Duration;

use libp2p::{
    Multiaddr, PeerId as Libp2pPeerId, Swarm, SwarmBuilder,
    dcutr, identify, kad, noise, ping, relay, tcp, yamux,
    swarm::NetworkBehaviour,
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
    // relay server is optional — only configured when RelayMode::Server
}

/// Events from the Ganglion swarm that the adapter translates to TransportEvents.
#[derive(Debug)]
pub enum SwarmEvent {
    PeerConnected(Libp2pPeerId),
    PeerDisconnected(Libp2pPeerId),
    RelayReservationAccepted(Libp2pPeerId),
    DirectConnectionUpgraded(Libp2pPeerId),
    IncomingStream {
        peer: Libp2pPeerId,
        protocol: String,
    },
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

    info!(%local_peer_id, "Building Ganglion swarm");

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
                identify::Config::new(
                    "/ganglion/0.1.0".into(),
                    keypair.public(),
                )
                .with_push_listen_addr_updates(true),
            );

            // Ping: keepalive and latency measurement
            let ping = ping::Behaviour::new(
                ping::Config::new().with_interval(Duration::from_secs(30)),
            );

            // Kademlia: peer routing through bootstrap nodes
            let mut kademlia = kad::Behaviour::new(
                local_peer_id,
                kad::store::MemoryStore::new(local_peer_id),
            );
            kademlia.set_mode(Some(kad::Mode::Client));

            // DCUtR: direct connection upgrade through relay
            let dcutr = dcutr::Behaviour::new(local_peer_id);

            Ok(GanglionBehaviour {
                identify,
                ping,
                kademlia,
                relay_client,
                dcutr,
            })
        })?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(Duration::from_secs(config.idle_timeout_secs))
        })
        .build();

    Ok((swarm, local_peer_id))
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
