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

    /// URL-pattern-allowlisted outbound HTTP (v0.5, ADR-025). Endpoints are
    /// URL globs with an access level: `read_only` permits GET/HEAD,
    /// `read_write` permits any method. Enforced per call against the
    /// component's own declaration, after deploy-time policy evaluation.
    #[serde(rename = "ganglion:http/egress")]
    HttpEgress {
        /// Interface version requested.
        version: String,
        /// URL patterns this capability requests access to.
        endpoints: Vec<AccessPattern>,
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
            Self::HttpEgress { .. } => "ganglion:http/egress",
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
            | Self::MetricsEmit { version, .. }
            | Self::HttpEgress { version, .. } => version,
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

/// Decide whether an HTTP request is permitted by a set of declared
/// endpoints (ADR-025). Pure and unit-tested — this is the imports-layer
/// per-call check for `ganglion:http/egress`.
///
/// Rules:
/// - The URL is matched with query string and fragment stripped: patterns
///   govern *where* a request may go (origin + path), never which
///   parameters it carries.
/// - A pattern whose access is `read_only` permits only GET and HEAD;
///   `read_write` permits any method.
/// - Method comparison is case-insensitive; matching is glob semantics via
///   the same engine the policy layer uses.
pub fn http_request_permitted(endpoints: &[AccessPattern], method: &str, url: &str) -> bool {
    let target = strip_query_and_fragment(url);
    let is_read = matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD");
    endpoints.iter().any(|ep| {
        let access_ok = match ep.access {
            RosAccess::ReadWrite => true,
            RosAccess::ReadOnly => is_read,
        };
        access_ok && glob_match::glob_match(&ep.pattern, target)
    })
}

/// Strip the query string and fragment from a URL, leaving scheme + origin +
/// path. `https://a/b?x=1#f` → `https://a/b`.
fn strip_query_and_fragment(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
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
    /// Credential slot names the component's manifest declared (#43).
    #[serde(default)]
    pub credential_slots: Vec<String>,
    /// The authenticated operator that deployed this capability, when known.
    /// `None` for capabilities loaded from disk after an agent restart (the
    /// deployer identity is not persisted) — the policy re-sync sweep (#37)
    /// then re-judges capability rules only.
    #[serde(default)]
    pub deployed_by: Option<crate::identity::PeerId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(pattern: &str, access: RosAccess) -> AccessPattern {
        AccessPattern {
            pattern: pattern.into(),
            access,
        }
    }

    #[test]
    fn http_permission_scopes_path_and_method() {
        let eps = vec![
            ep("https://api.example.com/v1/**", RosAccess::ReadOnly),
            ep("https://hooks.example.com/notify", RosAccess::ReadWrite),
        ];
        // Path-scoped read allowed; method gated by access level.
        assert!(http_request_permitted(
            &eps,
            "GET",
            "https://api.example.com/v1/nodes"
        ));
        assert!(http_request_permitted(
            &eps,
            "head",
            "https://api.example.com/v1/x/y"
        ));
        assert!(!http_request_permitted(
            &eps,
            "POST",
            "https://api.example.com/v1/nodes"
        ));
        // Outside the declared path: denied even for GET.
        assert!(!http_request_permitted(
            &eps,
            "GET",
            "https://api.example.com/v2/nodes"
        ));
        // Different host: denied.
        assert!(!http_request_permitted(
            &eps,
            "GET",
            "https://evil.example.com/v1/nodes"
        ));
        // read_write endpoint permits mutation, exactly at its pattern.
        assert!(http_request_permitted(
            &eps,
            "POST",
            "https://hooks.example.com/notify"
        ));
        assert!(!http_request_permitted(
            &eps,
            "POST",
            "https://hooks.example.com/other"
        ));
    }

    #[test]
    fn http_permission_strips_query_and_fragment() {
        let eps = vec![ep("https://api.example.com/v1/**", RosAccess::ReadOnly)];
        // Query/fragment never affect the decision (ADR-025).
        assert!(http_request_permitted(
            &eps,
            "GET",
            "https://api.example.com/v1/nodes?filter=all#frag"
        ));
        // ...and cannot be used to smuggle a path match.
        assert!(!http_request_permitted(
            &eps,
            "GET",
            "https://evil.example.com/x?u=https://api.example.com/v1/"
        ));
    }

    #[test]
    fn http_permission_empty_endpoints_denies_everything() {
        assert!(!http_request_permitted(
            &[],
            "GET",
            "https://api.example.com/"
        ));
    }
}
