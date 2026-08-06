/// Stream protocol identifiers for Ganglion.
/// All application-level traffic flows over libp2p streams multiplexed on the connection.
///
/// Control messages: capability deployment, invocation, presence, configuration.
pub const PROTOCOL_CONTROL: &str = "/ganglion/control/1.0";

/// Bidirectional stream between operator and an invoked capability.
pub const PROTOCOL_TOOL: &str = "/ganglion/tool/1.0";

/// High-volume artifact transfer (log bundles, rosbags, diagnostic tarballs).
pub const PROTOCOL_BULK: &str = "/ganglion/bulk/1.0";

/// Authenticated robot→operator event subscription (presence, policy
/// decisions, audit appends, connection state, heartbeats). See
/// [`crate::events`]. A subscriber opens this protocol and receives a
/// length-prefixed CBOR sequence of [`crate::events::AgentEvent`]s.
pub const PROTOCOL_EVENTS: &str = "/ganglion/events/1.0";

/// All known Ganglion stream protocols.
pub const ALL_PROTOCOLS: &[&str] = &[
    PROTOCOL_CONTROL,
    PROTOCOL_TOOL,
    PROTOCOL_BULK,
    PROTOCOL_EVENTS,
];

/// Protocol identifier type for type safety.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProtocolId(String);

impl ProtocolId {
    /// Construct a protocol identifier from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The protocol identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The control protocol identifier (`/ganglion/control/1.0`).
    pub fn control() -> Self {
        Self(PROTOCOL_CONTROL.into())
    }

    /// The tool protocol identifier (`/ganglion/tool/1.0`).
    pub fn tool() -> Self {
        Self(PROTOCOL_TOOL.into())
    }

    /// The bulk-transfer protocol identifier (`/ganglion/bulk/1.0`).
    pub fn bulk() -> Self {
        Self(PROTOCOL_BULK.into())
    }

    /// The event-subscription protocol identifier (`/ganglion/events/1.0`).
    pub fn events() -> Self {
        Self(PROTOCOL_EVENTS.into())
    }
}

impl std::fmt::Display for ProtocolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
