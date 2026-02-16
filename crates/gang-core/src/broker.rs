use serde::{Deserialize, Serialize};

use crate::error::BrokerError;

/// Brokers mediate access between WASM capabilities and privileged resources.
/// Each broker enforces policy and produces audit-ready I/O stats.

/// A request from a WASM capability to a Layer 3 broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    /// Which capability group this request targets.
    pub capability_group: String,
    /// The specific operation.
    pub operation: BrokerOperation,
}

/// Operations that brokers handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrokerOperation {
    // --- ROS Interface ---
    /// Subscribe to a topic (read).
    TopicSubscribe { topic: String },
    /// Publish to a topic (write).
    TopicPublish { topic: String, data: Vec<u8> },
    /// Call a service.
    ServiceCall { service: String, request: Vec<u8> },
    /// Get a parameter.
    ParamGet { name: String },
    /// Set a parameter.
    ParamSet { name: String, value: Vec<u8> },
    /// List all topics/services/parameters.
    RosList,

    // --- Logs ---
    /// List available log sources.
    LogSourceList,
    /// Stream log lines matching a pattern.
    LogStream { source: String, pattern: String },

    // --- Filesystem ---
    /// Read a file.
    FsRead { path: String },
    /// Write a file.
    FsWrite { path: String, data: Vec<u8> },
    /// List a directory.
    FsList { path: String },
    /// Stat a file.
    FsStat { path: String },

    // --- Diagnostics ---
    /// Collect system information.
    SystemInfo,
    /// List running processes.
    ProcessList,
    /// Collect network state.
    NetworkState,
}

/// Response from a broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResponse {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Response payload (operation-specific).
    pub data: Vec<u8>,
    /// Error message if !success.
    pub error: Option<String>,
    /// Bytes read from the resource.
    pub bytes_in: u64,
    /// Bytes written to the resource.
    pub bytes_out: u64,
}

/// The trait all Layer 3 brokers implement.
#[async_trait::async_trait]
pub trait CapabilityBroker: Send + Sync {
    /// Handle a request from a WASM capability.
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError>;

    /// The capability group this broker serves.
    fn capability_group(&self) -> &str;
}
