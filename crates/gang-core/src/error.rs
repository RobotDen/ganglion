//! Error types for the Ganglion core crate.
//!
//! Library code returns these `thiserror`-derived enums; the CLI layer wraps
//! them in `anyhow`.

use thiserror::Error;

/// Top-level error aggregating every fallible subsystem in `gang-core`.
#[derive(Debug, Error)]
pub enum GanglionError {
    /// A transport-layer failure.
    #[error("transport: {0}")]
    Transport(#[from] TransportError),

    /// A policy-engine failure or denial.
    #[error("policy: {0}")]
    Policy(#[from] PolicyError),

    /// An audit-log failure.
    #[error("audit: {0}")]
    Audit(#[from] AuditError),

    /// A manifest validation or decoding failure.
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),

    /// A capability execution failure.
    #[error("capability: {0}")]
    Capability(#[from] CapabilityError),

    /// A broker access failure.
    #[error("broker: {0}")]
    Broker(#[from] BrokerError),

    /// An underlying I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A catch-all error with a free-form message.
    #[error("{0}")]
    Other(String),
}

/// Failures raised by the connectivity/transport layer.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Dialing a peer failed.
    #[error("dial failed for peer {peer}: {reason}")]
    DialFailed {
        /// The peer that could not be reached.
        peer: String,
        /// Human-readable reason for the failure.
        reason: String,
    },

    /// An established connection was closed.
    #[error("connection closed: {0}")]
    ConnectionClosed(String),

    /// Stream protocol negotiation failed.
    #[error("protocol negotiation failed: {0}")]
    ProtocolNegotiation(String),

    /// No relay was available to reach the peer.
    #[error("relay unavailable: {0}")]
    RelayUnavailable(String),

    /// An operation exceeded its deadline.
    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),

    /// No transport could serve the given peer.
    #[error("no suitable transport for peer {0}")]
    NoTransport(String),
}

/// Failures raised by the default-deny policy engine.
#[derive(Debug, Error)]
pub enum PolicyError {
    /// The requested capability is not permitted by policy.
    #[error("capability {capability} not permitted by policy")]
    CapabilityDenied {
        /// The denied capability's qualified name.
        capability: String,
    },

    /// A requested pattern is broader than the policy allows.
    #[error("pattern {pattern} exceeds policy for {capability}")]
    PatternExceedsPolicy {
        /// The capability the pattern applies to.
        capability: String,
        /// The over-broad pattern that was rejected.
        pattern: String,
    },

    /// The peer is not authorized to deploy capabilities.
    #[error("peer {peer} not authorized to deploy capabilities")]
    PeerNotAuthorized {
        /// The unauthorized peer.
        peer: String,
    },

    /// The policy file could not be found.
    #[error("policy file not found: {0}")]
    PolicyNotFound(String),

    /// The policy document is malformed.
    #[error("invalid policy: {0}")]
    InvalidPolicy(String),
}

/// Failures raised by the append-only audit log.
#[derive(Debug, Error)]
pub enum AuditError {
    /// Writing a record to the log failed.
    #[error("failed to write audit record: {0}")]
    WriteFailed(String),

    /// The log could not be read or decoded.
    #[error("audit log corrupted: {0}")]
    Corrupted(String),

    /// The audit hash chain failed verification.
    #[error("audit chain integrity violation: {0}")]
    IntegrityViolation(String),
}

/// Failures raised while validating or decoding component manifests.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// The component ships without a signature.
    #[error("unsigned component")]
    Unsigned,

    /// The signature did not verify against the manifest.
    #[error("signature verification failed")]
    InvalidSignature,

    /// The signer is not present in the trust store.
    #[error("signer {peer} not in trust store")]
    UntrustedSigner {
        /// The untrusted signing peer.
        peer: String,
    },

    /// The manifest bytes could not be decoded.
    #[error("manifest decode failed: {0}")]
    DecodeFailed(String),

    /// The component bytes do not hash to the manifest's declared hash.
    #[error("component hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// The hash the manifest declares.
        expected: String,
        /// The hash actually computed from the component bytes.
        actual: String,
    },
}

/// Failures raised while executing a capability.
#[derive(Debug, Error)]
pub enum CapabilityError {
    /// The named capability is not installed on the robot.
    #[error("capability {0} not found on robot")]
    NotFound(String),

    /// The capability exceeded a configured resource limit.
    #[error("capability {name} exceeded resource limit: {limit}")]
    ResourceExhausted {
        /// The capability that exceeded its limit.
        name: String,
        /// Which limit was exceeded.
        limit: String,
    },

    /// The capability exceeded its wall-clock deadline.
    #[error("capability {name} timed out after {elapsed:?}")]
    Timeout {
        /// The capability that timed out.
        name: String,
        /// How long it ran before timing out.
        elapsed: std::time::Duration,
    },

    /// The WASM guest trapped during execution.
    #[error("capability {name} trapped: {message}")]
    Trapped {
        /// The capability that trapped.
        name: String,
        /// The trap message.
        message: String,
    },

    /// The WASM component could not be instantiated.
    #[error("wasm instantiation failed: {0}")]
    InstantiationFailed(String),
}

/// Failures raised by Layer 3 brokers.
#[derive(Debug, Error)]
pub enum BrokerError {
    /// The broker denied access to a resource.
    #[error("broker {broker} denied access to {resource}: {reason}")]
    AccessDenied {
        /// The broker that denied the request.
        broker: String,
        /// The resource that was requested.
        resource: String,
        /// Why access was denied.
        reason: String,
    },

    /// The broker is not available to serve requests.
    #[error("broker {broker} unavailable: {reason}")]
    Unavailable {
        /// The unavailable broker.
        broker: String,
        /// Why the broker is unavailable.
        reason: String,
    },

    /// The requested resource does not exist.
    #[error("resource not found: {0}")]
    ResourceNotFound(String),
}
