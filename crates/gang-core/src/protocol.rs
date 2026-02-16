/// Stream protocol identifiers for Ganglion.
/// All application-level traffic flows over libp2p streams multiplexed on the connection.

/// Control messages: capability deployment, invocation, presence, configuration.
pub const PROTOCOL_CONTROL: &str = "/ganglion/control/1.0";

/// Bidirectional stream between operator and an invoked capability.
pub const PROTOCOL_TOOL: &str = "/ganglion/tool/1.0";

/// High-volume artifact transfer (log bundles, rosbags, diagnostic tarballs).
pub const PROTOCOL_BULK: &str = "/ganglion/bulk/1.0";

/// All known Ganglion stream protocols.
pub const ALL_PROTOCOLS: &[&str] = &[PROTOCOL_CONTROL, PROTOCOL_TOOL, PROTOCOL_BULK];

/// Protocol identifier type for type safety.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProtocolId(String);

impl ProtocolId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn control() -> Self {
        Self(PROTOCOL_CONTROL.into())
    }

    pub fn tool() -> Self {
        Self(PROTOCOL_TOOL.into())
    }

    pub fn bulk() -> Self {
        Self(PROTOCOL_BULK.into())
    }
}

impl std::fmt::Display for ProtocolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
