use serde::{Deserialize, Serialize};

use crate::error::BrokerError;

/// Brokers mediate access between WASM capabilities and privileged resources.
/// Each broker enforces policy and produces audit-ready I/O stats.
///
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

    // --- Artifacts (v0.3) ---
    /// Publish a byte stream as a content-addressed artifact.
    ArtifactPublish {
        data: Vec<u8>,
        filename: Option<String>,
        content_type: Option<String>,
    },
    /// Check if an artifact exists by CID.
    ArtifactExists { cid: String },

    // --- Process (v0.4) ---
    /// Spawn a bounded subprocess.
    ProcessSpawn {
        command: String,
        args: Vec<String>,
        /// Working directory (within allowed paths).
        cwd: Option<String>,
        /// Environment variables to set.
        env: Vec<(String, String)>,
        /// Wall-clock timeout in seconds.
        timeout_secs: u64,
    },

    // --- Network Probe (v0.4) ---
    /// Ping a host and return latency.
    NetPing { host: String, count: u32 },
    /// DNS lookup.
    NetDnsLookup {
        hostname: String,
        record_type: String,
    },
    /// TCP port check.
    NetPortCheck {
        host: String,
        port: u16,
        timeout_secs: u64,
    },
    /// Traceroute to a host.
    NetTraceroute { host: String, max_hops: u32 },

    // --- Metrics (v0.4) ---
    /// Emit a metric value from a capability.
    MetricEmit {
        name: String,
        value: f64,
        unit: Option<String>,
        tags: Vec<(String, String)>,
    },
    /// Emit a batch of metrics.
    MetricEmitBatch { metrics: Vec<MetricPoint> },
}

/// A single metric data point for batch emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub tags: Vec<(String, String)>,
    /// Unix timestamp in milliseconds. 0 = use current time.
    pub timestamp_ms: u64,
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
