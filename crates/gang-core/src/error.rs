use thiserror::Error;

#[derive(Debug, Error)]
pub enum GanglionError {
    #[error("transport: {0}")]
    Transport(#[from] TransportError),

    #[error("policy: {0}")]
    Policy(#[from] PolicyError),

    #[error("audit: {0}")]
    Audit(#[from] AuditError),

    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),

    #[error("capability: {0}")]
    Capability(#[from] CapabilityError),

    #[error("broker: {0}")]
    Broker(#[from] BrokerError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("dial failed for peer {peer}: {reason}")]
    DialFailed { peer: String, reason: String },

    #[error("connection closed: {0}")]
    ConnectionClosed(String),

    #[error("protocol negotiation failed: {0}")]
    ProtocolNegotiation(String),

    #[error("relay unavailable: {0}")]
    RelayUnavailable(String),

    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("no suitable transport for peer {0}")]
    NoTransport(String),
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("capability {capability} not permitted by policy")]
    CapabilityDenied { capability: String },

    #[error("pattern {pattern} exceeds policy for {capability}")]
    PatternExceedsPolicy {
        capability: String,
        pattern: String,
    },

    #[error("peer {peer} not authorized to deploy capabilities")]
    PeerNotAuthorized { peer: String },

    #[error("policy file not found: {0}")]
    PolicyNotFound(String),

    #[error("invalid policy: {0}")]
    InvalidPolicy(String),
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("failed to write audit record: {0}")]
    WriteFailed(String),

    #[error("audit log corrupted: {0}")]
    Corrupted(String),
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("unsigned component")]
    Unsigned,

    #[error("signature verification failed")]
    InvalidSignature,

    #[error("signer {peer} not in trust store")]
    UntrustedSigner { peer: String },

    #[error("manifest decode failed: {0}")]
    DecodeFailed(String),

    #[error("component hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("capability {0} not found on robot")]
    NotFound(String),

    #[error("capability {name} exceeded resource limit: {limit}")]
    ResourceExhausted { name: String, limit: String },

    #[error("capability {name} timed out after {elapsed:?}")]
    Timeout {
        name: String,
        elapsed: std::time::Duration,
    },

    #[error("capability {name} trapped: {message}")]
    Trapped { name: String, message: String },

    #[error("wasm instantiation failed: {0}")]
    InstantiationFailed(String),
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("broker {broker} denied access to {resource}: {reason}")]
    AccessDenied {
        broker: String,
        resource: String,
        reason: String,
    },

    #[error("broker {broker} unavailable: {reason}")]
    Unavailable { broker: String, reason: String },

    #[error("resource not found: {0}")]
    ResourceNotFound(String),
}
