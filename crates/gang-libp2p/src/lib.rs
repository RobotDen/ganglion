mod adapter;
mod config;
mod relay;
mod swarm;

pub use adapter::{
    DialableIdentity, Libp2pTransportAdapter, identity_from_libp2p_str, libp2p_to_gang_peer_id,
};
pub use config::Libp2pConfig;
pub use relay::RelayMode;
pub use swarm::{GanglionCodec, GanglionProtocol, GanglionRequest};

use thiserror::Error;

/// Errors returned by this crate's public lifecycle entry points
/// (construction, event loop, relay setup).
///
/// The `TransportAdapter` trait methods still return the protocol-agnostic
/// `gang_core::error::TransportError`; this type is for libp2p-specific setup
/// and lifecycle failures that were previously surfaced as `anyhow::Error`.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The libp2p swarm could not be built.
    #[error("failed to build libp2p swarm: {0}")]
    SwarmBuild(String),

    /// The identity key could not be loaded or was malformed.
    #[error("invalid identity key: {0}")]
    InvalidKey(String),

    /// An I/O error occurred (e.g. reading the identity key file).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// `run_event_loop` was called more than once.
    #[error("event loop already running or already consumed")]
    EventLoopAlreadyRunning,

    /// Connecting to a configured relay failed.
    #[error("relay connection failed: {0}")]
    Relay(String),

    /// The swarm-owning task is no longer running.
    #[error("swarm task is not running")]
    SwarmTaskStopped,
}
