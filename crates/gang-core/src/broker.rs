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
#[non_exhaustive]
pub enum BrokerOperation {
    // --- ROS Interface ---
    /// Subscribe to a topic (read).
    TopicSubscribe {
        /// Topic name.
        topic: String,
    },
    /// Publish to a topic (write).
    TopicPublish {
        /// Topic name.
        topic: String,
        /// Serialized message payload.
        data: Vec<u8>,
    },
    /// Call a service.
    ServiceCall {
        /// Service name.
        service: String,
        /// Serialized request payload.
        request: Vec<u8>,
    },
    /// Get a parameter.
    ParamGet {
        /// Parameter name.
        name: String,
    },
    /// Set a parameter.
    ParamSet {
        /// Parameter name.
        name: String,
        /// Serialized parameter value.
        value: Vec<u8>,
    },
    /// List all topics/services/parameters.
    RosList,

    // --- Logs ---
    /// List available log sources.
    LogSourceList,
    /// Stream log lines matching a pattern.
    LogStream {
        /// Log source identifier.
        source: String,
        /// Filter pattern.
        pattern: String,
    },

    // --- Filesystem ---
    /// Read a file.
    FsRead {
        /// Path to read.
        path: String,
    },
    /// Write a file.
    FsWrite {
        /// Path to write.
        path: String,
        /// Bytes to write.
        data: Vec<u8>,
    },
    /// List a directory.
    FsList {
        /// Directory path.
        path: String,
    },
    /// Stat a file.
    FsStat {
        /// Path to stat.
        path: String,
    },

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
        /// Artifact bytes.
        data: Vec<u8>,
        /// Optional original filename.
        filename: Option<String>,
        /// Optional MIME content type.
        content_type: Option<String>,
    },
    /// Check if an artifact exists by CID.
    ArtifactExists {
        /// The content identifier to check.
        cid: String,
    },

    // --- Process (v0.4) ---
    /// Spawn a bounded subprocess.
    ProcessSpawn {
        /// Command to run (subject to allowlist).
        command: String,
        /// Command arguments.
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
    NetPing {
        /// Target host.
        host: String,
        /// Number of pings to send.
        count: u32,
    },
    /// DNS lookup.
    NetDnsLookup {
        /// Hostname to resolve.
        hostname: String,
        /// DNS record type (e.g. "A", "AAAA").
        record_type: String,
    },
    /// TCP port check.
    NetPortCheck {
        /// Target host.
        host: String,
        /// Target port.
        port: u16,
        /// Connection timeout in seconds.
        timeout_secs: u64,
    },
    /// Traceroute to a host.
    NetTraceroute {
        /// Target host.
        host: String,
        /// Maximum number of hops.
        max_hops: u32,
    },

    // --- Metrics (v0.4) ---
    /// Emit a metric value from a capability.
    MetricEmit {
        /// Metric name.
        name: String,
        /// Metric value.
        value: f64,
        /// Optional unit.
        unit: Option<String>,
        /// Metric tags as key/value pairs.
        tags: Vec<(String, String)>,
    },
    /// Emit a batch of metrics.
    MetricEmitBatch {
        /// The metric points to emit.
        metrics: Vec<MetricPoint>,
    },

    // --- HTTP egress (v0.5, ADR-025) ---
    /// Perform an outbound HTTP request. The URL and method have already been
    /// validated against the calling component's declared endpoints at the
    /// imports layer; the broker enforces mechanics (scheme, size and time
    /// bounds, no redirect following, header hygiene).
    HttpRequest {
        /// HTTP method (uppercase).
        method: String,
        /// Absolute URL.
        url: String,
        /// Request headers.
        headers: Vec<(String, String)>,
        /// Request body (empty for body-less methods).
        body: Vec<u8>,
    },
}

/// Response payload for [`BrokerOperation::HttpRequest`], CBOR-encoded into
/// [`CapabilityResponse::data`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponseData {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body, bounded by the broker's size cap.
    pub body: Vec<u8>,
}

/// A single metric data point for batch emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    /// Metric name.
    pub name: String,
    /// Metric value.
    pub value: f64,
    /// Optional unit.
    pub unit: Option<String>,
    /// Metric tags as key/value pairs.
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
