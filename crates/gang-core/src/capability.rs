use serde::{Deserialize, Serialize};

/// Capability groups defined by Ganglion's WIT interface.
/// A component declares which capability groups it needs; the policy engine
/// evaluates the declaration against active policy at load time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityGroup {
    /// Read-only and read-write access to ROS 2 topics, services, and parameters.
    #[serde(rename = "ganglion:ros/interface")]
    RosInterface {
        version: String,
        /// Topic/service/parameter patterns this capability requests access to.
        patterns: Vec<AccessPattern>,
    },

    /// Read access to system logs, journald, and ROS log files.
    #[serde(rename = "ganglion:logs/stream")]
    LogStream {
        version: String,
        /// Log source patterns.
        patterns: Vec<String>,
    },

    /// Bounded filesystem access.
    #[serde(rename = "ganglion:fs/bounded")]
    FsBounded {
        version: String,
        /// Path patterns with explicit read/write/execute flags.
        paths: Vec<FsAccessPattern>,
    },

    /// Structured diagnostic collection primitives.
    #[serde(rename = "ganglion:diagnostics/collect")]
    DiagnosticsCollect {
        version: String,
    },

    /// Content-addressed artifact publishing (v0.3).
    #[serde(rename = "ganglion:artifacts/publish")]
    ArtifactsPublish {
        version: String,
    },
}

impl CapabilityGroup {
    pub fn name(&self) -> &str {
        match self {
            Self::RosInterface { .. } => "ganglion:ros/interface",
            Self::LogStream { .. } => "ganglion:logs/stream",
            Self::FsBounded { .. } => "ganglion:fs/bounded",
            Self::DiagnosticsCollect { .. } => "ganglion:diagnostics/collect",
            Self::ArtifactsPublish { .. } => "ganglion:artifacts/publish",
        }
    }

    pub fn version(&self) -> &str {
        match self {
            Self::RosInterface { version, .. }
            | Self::LogStream { version, .. }
            | Self::FsBounded { version, .. }
            | Self::DiagnosticsCollect { version, .. }
            | Self::ArtifactsPublish { version, .. } => version,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosAccess {
    ReadOnly,
    ReadWrite,
}

/// Filesystem access pattern with permission flags.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FsAccessPattern {
    /// Glob pattern for file paths.
    pub pattern: String,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

/// Describes a capability installed on a robot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledCapability {
    pub name: String,
    pub version: String,
    pub author_peer_id: crate::identity::PeerId,
    pub declared_capabilities: Vec<CapabilityGroup>,
    pub component_hash: String,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    /// Path to the .wasm component on disk.
    pub component_path: std::path::PathBuf,
    /// Path to the manifest on disk.
    pub manifest_path: std::path::PathBuf,
}
