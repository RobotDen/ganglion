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

/// Check that a broker operation stays inside the scope the calling
/// component itself declared (SEC-19).
///
/// Brokers enforce the agent-wide rules (the filesystem jail, allowed log
/// sources). This narrows each call further to the patterns in the
/// component's signed manifest, which is what deploy-time policy approved:
/// a component that declared read access to `/data/maps/**` cannot write
/// there, and cannot touch anything else the agent could reach. An empty
/// declared pattern list permits nothing. Operations without a target
/// (listings, system info, artifacts) are not narrowed here.
pub fn operation_in_declared_scope(
    declared: &[CapabilityGroup],
    group: &str,
    operation: &crate::broker::BrokerOperation,
) -> Result<(), String> {
    use crate::broker::BrokerOperation as Op;

    let fs_paths = || {
        declared.iter().find_map(|g| match g {
            CapabilityGroup::FsBounded { paths, .. } => Some(paths.as_slice()),
            _ => None,
        })
    };
    let ros_patterns = || {
        declared.iter().find_map(|g| match g {
            CapabilityGroup::RosInterface { patterns, .. } => Some(patterns.as_slice()),
            _ => None,
        })
    };
    let fs_check = |path: &str, write: bool| -> Result<(), String> {
        let p = std::path::Path::new(path);
        if !p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!(
                "path '{path}' must be absolute and must not contain '..'"
            ));
        }
        let ok = fs_paths().unwrap_or(&[]).iter().any(|fp| {
            glob_match::glob_match(&fp.pattern, path) && if write { fp.write } else { fp.read }
        });
        if ok {
            Ok(())
        } else {
            Err(format!(
                "{} of '{path}' is outside the paths this capability declared",
                if write { "write" } else { "read" }
            ))
        }
    };
    let ros_check = |name: &str, write: bool| -> Result<(), String> {
        let ok = ros_patterns().unwrap_or(&[]).iter().any(|ap| {
            glob_match::glob_match(&ap.pattern, name)
                && (!write || ap.access == RosAccess::ReadWrite)
        });
        if ok {
            Ok(())
        } else {
            Err(format!(
                "{} access to '{name}' is outside the ROS patterns this capability declared",
                if write { "write" } else { "read" }
            ))
        }
    };

    match (group, operation) {
        (
            "ganglion:fs/bounded",
            Op::FsRead { path } | Op::FsList { path } | Op::FsStat { path },
        ) => fs_check(path, false),
        ("ganglion:fs/bounded", Op::FsWrite { path, .. }) => fs_check(path, true),
        ("ganglion:ros/interface", Op::TopicSubscribe { topic }) => ros_check(topic, false),
        ("ganglion:ros/interface", Op::ParamGet { name }) => ros_check(name, false),
        ("ganglion:ros/interface", Op::TopicPublish { topic, .. }) => ros_check(topic, true),
        ("ganglion:ros/interface", Op::ServiceCall { service, .. }) => ros_check(service, true),
        ("ganglion:ros/interface", Op::ParamSet { name, .. }) => ros_check(name, true),
        ("ganglion:logs/stream", Op::LogStream { source, .. }) => {
            let ok = declared.iter().any(|g| match g {
                CapabilityGroup::LogStream { patterns, .. } => patterns
                    .iter()
                    .any(|pat| glob_match::glob_match(pat, source)),
                _ => false,
            });
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "log source '{source}' is outside the sources this capability declared"
                ))
            }
        }
        _ => Ok(()),
    }
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

    // --- SEC-19: broker calls confined to the declared scope ---

    use crate::broker::BrokerOperation as Op;

    fn fs_decl(pattern: &str, write: bool) -> Vec<CapabilityGroup> {
        vec![CapabilityGroup::FsBounded {
            version: "1.0".into(),
            paths: vec![FsAccessPattern {
                pattern: pattern.into(),
                read: true,
                write,
                execute: false,
            }],
        }]
    }

    const FS: &str = "ganglion:fs/bounded";

    #[test]
    fn fs_read_inside_declared_path_is_allowed() {
        let d = fs_decl("/data/maps/**", false);
        let op = Op::FsRead {
            path: "/data/maps/a.yaml".into(),
        };
        assert!(operation_in_declared_scope(&d, FS, &op).is_ok());
    }

    #[test]
    fn fs_read_outside_declared_path_is_denied() {
        // The key-theft path: a component declared for /data/maps/** must not
        // read the agent's identity key, whatever the agent-wide jail allows.
        let d = fs_decl("/data/maps/**", false);
        let op = Op::FsRead {
            path: "/home/robot/.gang/identity.key".into(),
        };
        assert!(operation_in_declared_scope(&d, FS, &op).is_err());
    }

    #[test]
    fn fs_write_needs_a_writable_declaration() {
        let ro = fs_decl("/data/maps/**", false);
        let write = Op::FsWrite {
            path: "/data/maps/a.yaml".into(),
            data: vec![],
        };
        assert!(operation_in_declared_scope(&ro, FS, &write).is_err());
        let rw = fs_decl("/data/maps/**", true);
        assert!(operation_in_declared_scope(&rw, FS, &write).is_ok());
    }

    #[test]
    fn fs_parent_dir_and_relative_paths_are_denied() {
        let d = fs_decl("/data/**", true);
        for path in ["/data/../etc/shadow", "data/x", "../x"] {
            let op = Op::FsRead { path: path.into() };
            assert!(operation_in_declared_scope(&d, FS, &op).is_err(), "{path}");
        }
    }

    #[test]
    fn group_declared_without_patterns_permits_nothing() {
        let d = vec![CapabilityGroup::FsBounded {
            version: "1.0".into(),
            paths: vec![],
        }];
        let op = Op::FsRead {
            path: "/anything".into(),
        };
        assert!(operation_in_declared_scope(&d, FS, &op).is_err());
    }

    #[test]
    fn ros_write_needs_read_write_pattern() {
        let d = vec![CapabilityGroup::RosInterface {
            version: "1.0".into(),
            patterns: vec![ep("/diagnostics/**", RosAccess::ReadOnly)],
        }];
        let g = "ganglion:ros/interface";
        let sub = Op::TopicSubscribe {
            topic: "/diagnostics/cpu".into(),
        };
        assert!(operation_in_declared_scope(&d, g, &sub).is_ok());
        let publish = Op::TopicPublish {
            topic: "/diagnostics/cpu".into(),
            data: vec![],
        };
        assert!(operation_in_declared_scope(&d, g, &publish).is_err());
        let other = Op::TopicSubscribe {
            topic: "/cmd_vel".into(),
        };
        assert!(operation_in_declared_scope(&d, g, &other).is_err());
    }

    #[test]
    fn log_source_must_match_declared_patterns() {
        let d = vec![CapabilityGroup::LogStream {
            version: "1.0".into(),
            patterns: vec!["journald/**".into()],
        }];
        let g = "ganglion:logs/stream";
        let ok = Op::LogStream {
            source: "journald/nav2".into(),
            pattern: String::new(),
        };
        assert!(operation_in_declared_scope(&d, g, &ok).is_ok());
        let bad = Op::LogStream {
            source: "/var/log/auth.log".into(),
            pattern: String::new(),
        };
        assert!(operation_in_declared_scope(&d, g, &bad).is_err());
    }
}
