use serde::{Deserialize, Serialize};

/// Capability groups defined by Ganglion's WIT interface.
/// A component declares which capability groups it needs; the policy engine
/// evaluates the declaration against active policy at load time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityGroup {
    /// Read-only and read-write access to ROS 2 topics, services, and parameters.
    #[serde(rename = "ganglion:ros/interface")]
    RosInterface {
        /// Interface version requested.
        version: String,
        /// Topic/service/parameter patterns this capability requests access to.
        patterns: Vec<AccessPattern>,
    },

    /// Read access to system logs, journald, and ROS log files.
    #[serde(rename = "ganglion:logs/stream")]
    LogStream {
        /// Interface version requested.
        version: String,
        /// Log source patterns.
        patterns: Vec<String>,
    },

    /// Bounded filesystem access.
    #[serde(rename = "ganglion:fs/bounded")]
    FsBounded {
        /// Interface version requested.
        version: String,
        /// Path patterns with explicit read/write/execute flags.
        paths: Vec<FsAccessPattern>,
    },

    /// Structured diagnostic collection primitives.
    #[serde(rename = "ganglion:diagnostics/collect")]
    DiagnosticsCollect {
        /// Interface version requested.
        version: String,
    },

    /// Content-addressed artifact publishing (v0.3).
    #[serde(rename = "ganglion:artifacts/publish")]
    ArtifactsPublish {
        /// Interface version requested.
        version: String,
    },

    /// Bounded subprocess invocation (v0.4).
    #[serde(rename = "ganglion:process/spawn")]
    ProcessSpawn {
        /// Interface version requested.
        version: String,
        /// Allowlisted command patterns.
        allowed_commands: Vec<String>,
    },

    /// Structured network probing primitives (v0.4).
    #[serde(rename = "ganglion:network/probe")]
    NetworkProbe {
        /// Interface version requested.
        version: String,
    },

    /// Structured metric emission from capabilities (v0.4).
    #[serde(rename = "ganglion:metrics/emit")]
    MetricsEmit {
        /// Interface version requested.
        version: String,
    },
}

impl CapabilityGroup {
    /// The capability group's qualified interface name (without version).
    pub fn name(&self) -> &str {
        match self {
            Self::RosInterface { .. } => "ganglion:ros/interface",
            Self::LogStream { .. } => "ganglion:logs/stream",
            Self::FsBounded { .. } => "ganglion:fs/bounded",
            Self::DiagnosticsCollect { .. } => "ganglion:diagnostics/collect",
            Self::ArtifactsPublish { .. } => "ganglion:artifacts/publish",
            Self::ProcessSpawn { .. } => "ganglion:process/spawn",
            Self::NetworkProbe { .. } => "ganglion:network/probe",
            Self::MetricsEmit { .. } => "ganglion:metrics/emit",
        }
    }

    /// The requested interface version for this capability group.
    pub fn version(&self) -> &str {
        match self {
            Self::RosInterface { version, .. }
            | Self::LogStream { version, .. }
            | Self::FsBounded { version, .. }
            | Self::DiagnosticsCollect { version, .. }
            | Self::ArtifactsPublish { version, .. }
            | Self::ProcessSpawn { version, .. }
            | Self::NetworkProbe { version, .. }
            | Self::MetricsEmit { version, .. } => version,
        }
    }

    /// Full identifier with version (e.g., "ganglion:ros/interface@1.0").
    pub fn qualified_name(&self) -> String {
        format!("{}@{}", self.name(), self.version())
    }
}

/// Access pattern for ROS interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccessPattern {
    /// Glob pattern for topic/service/parameter names.
    pub pattern: String,
    /// Access mode.
    pub access: RosAccess,
}

/// Access mode for a ROS interface pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosAccess {
    /// Read-only access (subscribe / get).
    ReadOnly,
    /// Read-write access (publish / set / call).
    ReadWrite,
}

/// Filesystem access pattern with permission flags.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FsAccessPattern {
    /// Glob pattern for file paths.
    pub pattern: String,
    /// Whether reads are permitted.
    pub read: bool,
    /// Whether writes are permitted.
    pub write: bool,
    /// Whether execution is permitted.
    pub execute: bool,
}

/// Describes a capability installed on a robot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledCapability {
    /// Capability name.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// Peer ID of the capability author.
    pub author_peer_id: crate::identity::PeerId,
    /// Capability groups the component declared.
    pub declared_capabilities: Vec<CapabilityGroup>,
    /// Blake3 hash of the installed component.
    pub component_hash: String,
    /// When the capability was installed.
    pub installed_at: chrono::DateTime<chrono::Utc>,
    /// Path to the .wasm component on disk.
    pub component_path: std::path::PathBuf,
    /// Path to the manifest on disk.
    pub manifest_path: std::path::PathBuf,
}
