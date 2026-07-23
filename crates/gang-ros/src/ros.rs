use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tracing::warn;

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Maximum output size from a `ros2` CLI invocation (1 MiB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// Default timeout for `ros2` graph introspection commands.
const ROS2_CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Short timeout used only for the availability probe (`ros2 --version`).
const ROS2_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// ROS 2 interface broker — mediates WASM capability access to the ROS 2 graph.
///
/// v0.1 implementation shells out to the `ros2` CLI for graph introspection.
/// Future versions will add rosbridge WebSocket transport and/or rclrs for
/// direct integration.
///
/// This broker enforces pattern-based access gating: each operation is checked
/// against the capability's declared topic/service/parameter patterns before
/// executing.
pub struct RosInterfaceBroker {
    /// Allowed topic/service/parameter patterns.
    allowed_patterns: Vec<RosAccessRule>,
    /// Whether the `ros2` CLI tools are accessible (proxy for ROS 2 running).
    ros2_available: bool,
}

#[derive(Debug, Clone)]
pub struct RosAccessRule {
    pub pattern: String,
    pub read: bool,
    pub write: bool,
}

impl RosInterfaceBroker {
    pub fn new(allowed_patterns: Vec<RosAccessRule>) -> Self {
        // The availability probe is async (tokio). Run it once, at
        // construction, on a throwaway thread with its own current-thread
        // runtime so this stays a synchronous constructor and never blocks
        // (or panics inside) a caller's async executor.
        let ros2_available = std::thread::spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map(|rt| rt.block_on(check_ros2_available()))
                .unwrap_or(false)
        })
        .join()
        .unwrap_or(false);

        Self {
            allowed_patterns,
            ros2_available,
        }
    }

    /// Create a broker with an explicit `ros2_available` flag.
    /// Useful for testing or when the caller already knows the connection state.
    #[cfg(test)]
    fn with_ros2_flag(allowed_patterns: Vec<RosAccessRule>, ros2_available: bool) -> Self {
        Self {
            allowed_patterns,
            ros2_available,
        }
    }

    /// Filter a list of ROS 2 resource names to only those matching at least
    /// one of the broker's allowed patterns (read access is sufficient).
    fn filter_by_allowed_patterns(&self, names: Vec<String>) -> Vec<String> {
        names
            .into_iter()
            .filter(|name| {
                self.allowed_patterns
                    .iter()
                    .any(|rule| glob_match::glob_match(&rule.pattern, name))
            })
            .collect()
    }

    fn check_access(&self, resource: &str, needs_write: bool) -> Result<(), BrokerError> {
        validate_ros_name(resource)?;
        for rule in &self.allowed_patterns {
            if glob_match::glob_match(&rule.pattern, resource) {
                if needs_write && !rule.write {
                    return Err(BrokerError::AccessDenied {
                        broker: "ros".into(),
                        resource: resource.into(),
                        reason: "write not permitted".into(),
                    });
                }
                return Ok(());
            }
        }

        Err(BrokerError::AccessDenied {
            broker: "ros".into(),
            resource: resource.into(),
            reason: "topic/service/parameter does not match any allowed pattern".into(),
        })
    }
}

#[async_trait]
impl CapabilityBroker for RosInterfaceBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::RosList => {
                // Use ros2 CLI to list topics, services, and nodes, then
                // filter each list to only resources matching the broker's
                // allowed patterns so unprivileged components cannot
                // enumerate the full ROS 2 graph.
                let topics = self.filter_by_allowed_patterns(ros2_topic_list().await);
                let services = self.filter_by_allowed_patterns(ros2_service_list().await);
                let nodes = self.filter_by_allowed_patterns(ros2_node_list().await);

                let listing = RosListing {
                    topics,
                    services,
                    nodes,
                };

                let data = serde_json::to_vec(&listing).map_err(|e| BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: true,
                    data,
                    error: None,
                    bytes_in: 0,
                    bytes_out,
                })
            }
            BrokerOperation::TopicSubscribe { ref topic } => {
                self.check_access(topic, false)?;

                if !self.ros2_available {
                    return Err(BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: "ros2 CLI not available — is ROS 2 running?".into(),
                    });
                }

                // In full implementation: subscribe via rosbridge WebSocket or
                // rclrs and stream messages back. For now, return current topic info.
                let info = ros2_topic_info(topic).await;
                let data = serde_json::to_vec(&info).map_err(|e| BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: true,
                    data,
                    error: None,
                    bytes_in: 0,
                    bytes_out,
                })
            }
            BrokerOperation::ServiceCall {
                ref service,
                ref request,
            } => {
                self.check_access(service, true)?;

                if !self.ros2_available {
                    return Err(BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: "ros2 CLI not available".into(),
                    });
                }

                // CODE-12: rosbridge WebSocket transport is not yet wired up,
                // so this operation cannot actually be dispatched. Report it
                // honestly as unavailable rather than returning a fake
                // success that a caller would mistake for a completed call.
                let _ = request;
                Err(BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: "service calls not implemented — rosbridge transport pending".into(),
                })
            }
            BrokerOperation::ParamGet { ref name } => {
                self.check_access(name, false)?;

                if !self.ros2_available {
                    return Err(BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: "ros2 CLI not available — is ROS 2 running?".into(),
                    });
                }

                // CODE-12: parameter reads over rosbridge are not implemented
                // yet. Return unavailable instead of a fabricated success.
                Err(BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: "parameter get not implemented — rosbridge transport pending".into(),
                })
            }
            BrokerOperation::ParamSet {
                ref name,
                ref value,
            } => {
                self.check_access(name, true)?;

                if !self.ros2_available {
                    return Err(BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: "ros2 CLI not available — is ROS 2 running?".into(),
                    });
                }

                // CODE-12: parameter writes over rosbridge are not implemented
                // yet. Return unavailable instead of a fabricated success.
                let _ = value;
                Err(BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: "parameter set not implemented — rosbridge transport pending".into(),
                })
            }
            _ => Err(BrokerError::AccessDenied {
                broker: "ros".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by ROS interface broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:ros/interface"
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RosListing {
    pub topics: Vec<String>,
    pub services: Vec<String>,
    pub nodes: Vec<String>,
}

/// Validate that a ROS 2 resource name contains only safe characters.
///
/// ROS 2 names use alphanumerics, underscores, hyphens, and forward slashes.
/// Rejecting anything else prevents shell metacharacter injection even though
/// `Command::new().args()` does not invoke a shell.
fn validate_ros_name(name: &str) -> Result<(), BrokerError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '-')
    {
        return Err(BrokerError::AccessDenied {
            broker: "ros".into(),
            resource: name.into(),
            reason: "invalid ROS 2 resource name".into(),
        });
    }
    Ok(())
}

/// Run a `ros2` CLI sub-command with a wall-clock timeout and output size
/// limit.  Returns the trimmed stdout on success.
///
/// Fully async: uses `tokio::process` + `tokio::time::timeout`, draining
/// stdout/stderr concurrently to avoid pipe-buffer deadlock, so it never
/// blocks the async executor. On timeout the child is explicitly killed and
/// awaited (reaped) to avoid leaving a zombie.
///
/// On failure the stderr is logged at `warn!` level via `tracing`.
async fn ros2_command_with_timeout(args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = tokio::process::Command::new("ros2")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Belt-and-suspenders reaping if this future is cancelled/dropped.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn ros2: {e}"))?;

    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();

    let collect = async {
        // Drain both pipes concurrently while the process runs so a full
        // pipe buffer can never deadlock the wait, then reap the child.
        let (r_out, r_err) = tokio::join!(
            stdout_pipe.read_to_end(&mut stdout_buf),
            stderr_pipe.read_to_end(&mut stderr_buf),
        );
        r_out.map_err(|e| format!("failed to read stdout: {e}"))?;
        r_err.map_err(|e| format!("failed to read stderr: {e}"))?;
        child
            .wait()
            .await
            .map_err(|e| format!("error waiting for ros2: {e}"))
    };

    // Bind to a local first so the `collect` future (which mutably borrows
    // `child` and the buffers) is dropped before the match arms run.
    let result = tokio::time::timeout(timeout, collect).await;
    match result {
        Ok(Ok(status)) => {
            if !status.success() {
                let stderr = String::from_utf8_lossy(&stderr_buf);
                warn!(
                    args = ?args,
                    status = %status,
                    stderr = %stderr,
                    "ros2 command failed"
                );
                return Err(format!("ros2 exited with {status}: {stderr}"));
            }

            if stdout_buf.len() > MAX_OUTPUT_BYTES {
                warn!(
                    args = ?args,
                    bytes = stdout_buf.len(),
                    limit = MAX_OUTPUT_BYTES,
                    "ros2 output exceeded size limit"
                );
                return Err("ros2 output exceeded 1 MiB limit".into());
            }

            Ok(String::from_utf8_lossy(&stdout_buf).to_string())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout: explicitly kill AND wait to reap the child so it does
            // not become a zombie or outlive the reported timeout.
            let _ = child.start_kill();
            let _ = child.wait().await;
            warn!(
                args = ?args,
                timeout = ?timeout,
                "ros2 command timed out — killed child process"
            );
            Err(format!("ros2 command timed out after {timeout:?}"))
        }
    }
}

/// Check whether the `ros2` CLI tools are accessible, which is a proxy for
/// whether ROS 2 is installed and the environment is sourced.
///
/// Uses `ros2 --version` with a 2-second timeout — much lighter than listing
/// the full topic graph.
///
/// NOTE: When WebSocket rosbridge transport is added, this should check
/// port 9090 connectivity instead (or in addition).
async fn check_ros2_available() -> bool {
    ros2_command_with_timeout(&["--version"], ROS2_PROBE_TIMEOUT)
        .await
        .is_ok()
}

/// Parse newline-delimited CLI output into a `Vec<String>`, skipping blanks.
fn lines_to_vec(output: &str) -> Vec<String> {
    output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// List all ROS 2 topics by shelling out to `ros2 topic list`.
///
/// Uses the ros2 CLI directly. This is intentional: the broker runs as a
/// privileged Layer 3 host process. Rosbridge WebSocket transport is planned
/// for future versions.
async fn ros2_topic_list() -> Vec<String> {
    ros2_command_with_timeout(&["topic", "list"], ROS2_CMD_TIMEOUT)
        .await
        .map(|s| lines_to_vec(&s))
        .unwrap_or_default()
}

/// List all ROS 2 services by shelling out to `ros2 service list`.
///
/// Uses the ros2 CLI directly. Rosbridge WebSocket transport is planned
/// for future versions.
async fn ros2_service_list() -> Vec<String> {
    ros2_command_with_timeout(&["service", "list"], ROS2_CMD_TIMEOUT)
        .await
        .map(|s| lines_to_vec(&s))
        .unwrap_or_default()
}

/// List all ROS 2 nodes by shelling out to `ros2 node list`.
///
/// Uses the ros2 CLI directly. Rosbridge WebSocket transport is planned
/// for future versions.
async fn ros2_node_list() -> Vec<String> {
    ros2_command_with_timeout(&["node", "list"], ROS2_CMD_TIMEOUT)
        .await
        .map(|s| lines_to_vec(&s))
        .unwrap_or_default()
}

/// Get verbose info for a single ROS 2 topic by shelling out to
/// `ros2 topic info <topic> -v`.
///
/// The `topic` name must already be validated by `validate_ros_name` before
/// calling this function.
///
/// Uses the ros2 CLI directly. Rosbridge WebSocket transport is planned
/// for future versions.
async fn ros2_topic_info(topic: &str) -> serde_json::Value {
    match ros2_command_with_timeout(&["topic", "info", topic, "-v"], ROS2_CMD_TIMEOUT).await {
        Ok(text) => serde_json::json!({
            "topic": topic,
            "info": text,
            "available": true,
        }),
        Err(e) => serde_json::json!({
            "topic": topic,
            "error": e,
            "available": false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw_broker(ros2: bool) -> RosInterfaceBroker {
        RosInterfaceBroker::with_ros2_flag(
            vec![RosAccessRule {
                pattern: "/params/**".into(),
                read: true,
                write: true,
            }],
            ros2,
        )
    }

    fn ro_broker(ros2: bool) -> RosInterfaceBroker {
        RosInterfaceBroker::with_ros2_flag(
            vec![RosAccessRule {
                pattern: "/params/**".into(),
                read: true,
                write: false,
            }],
            ros2,
        )
    }

    fn param_set_request(name: &str, value: Vec<u8>) -> CapabilityRequest {
        CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ParamSet {
                name: name.into(),
                value,
            },
        }
    }

    #[tokio::test]
    async fn param_set_unimplemented_returns_unavailable() {
        // CODE-12: ParamSet is not implemented (rosbridge pending). With write
        // access and ros2 available, it must report Unavailable, not a fake
        // success.
        let broker = rw_broker(true);
        let req = param_set_request("/params/max_speed", vec![0x42, 0x28, 0x00, 0x00]);
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::Unavailable { reason, .. } => {
                assert!(reason.contains("not implemented"), "reason: {reason}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn param_set_denied_without_write_access() {
        let broker = ro_broker(true);
        let req = param_set_request("/params/max_speed", vec![0x01]);
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::AccessDenied { reason, .. } => {
                assert!(reason.contains("write not permitted"));
            }
            other => panic!("expected AccessDenied, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn param_set_denied_pattern_mismatch() {
        let broker = rw_broker(true);
        let req = param_set_request("/other/param", vec![0x01]);
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));
    }

    #[tokio::test]
    async fn param_set_unavailable_without_ros2() {
        let broker = rw_broker(false);
        let req = param_set_request("/params/max_speed", vec![0x01]);
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::Unavailable { .. }));
    }

    // --- ParamGet tests ---

    #[tokio::test]
    async fn param_get_unimplemented_returns_unavailable() {
        // CODE-12: ParamGet is not implemented; must report Unavailable rather
        // than a fabricated success payload.
        let broker = rw_broker(true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ParamGet {
                name: "/params/max_speed".into(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::Unavailable { reason, .. } => {
                assert!(reason.contains("not implemented"), "reason: {reason}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn param_get_unavailable_without_ros2() {
        let broker = rw_broker(false);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ParamGet {
                name: "/params/max_speed".into(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn param_get_denied_pattern_mismatch() {
        let broker = rw_broker(true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ParamGet {
                name: "/forbidden/param".into(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));
    }

    // --- ServiceCall tests ---

    #[tokio::test]
    async fn service_call_unimplemented_returns_unavailable() {
        // CODE-12: ServiceCall is not implemented; must report Unavailable
        // rather than a fabricated "accepted" success.
        let broker = rw_broker(true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ServiceCall {
                service: "/params/set_bool".into(),
                request: b"{}".to_vec(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::Unavailable { reason, .. } => {
                assert!(reason.contains("not implemented"), "reason: {reason}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_call_unavailable_without_ros2() {
        let broker = rw_broker(false);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ServiceCall {
                service: "/params/set_bool".into(),
                request: vec![],
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn service_call_denied_without_write_permission() {
        let broker = ro_broker(true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ServiceCall {
                service: "/params/set_bool".into(),
                request: vec![],
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::AccessDenied { reason, .. } => {
                assert!(reason.contains("write not permitted"));
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_call_denied_pattern_mismatch() {
        let broker = rw_broker(true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ServiceCall {
                service: "/other/service".into(),
                request: vec![],
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));
    }

    // -------------------------------------------------------------------
    // Helpers for the broader test suite
    // -------------------------------------------------------------------

    fn rule(pattern: &str, read: bool, write: bool) -> RosAccessRule {
        RosAccessRule {
            pattern: pattern.to_string(),
            read,
            write,
        }
    }

    fn make_broker(patterns: Vec<RosAccessRule>, ros2: bool) -> RosInterfaceBroker {
        RosInterfaceBroker::with_ros2_flag(patterns, ros2)
    }

    // -------------------------------------------------------------------
    // Constructor tests
    // -------------------------------------------------------------------

    #[test]
    fn test_ros_broker_default() {
        let broker = make_broker(vec![], false);
        assert!(!broker.ros2_available);
        assert!(broker.allowed_patterns.is_empty());
    }

    #[test]
    fn test_ros_broker_with_patterns() {
        let patterns = vec![rule("/cmd_vel", true, true), rule("/scan", true, false)];
        let broker = make_broker(patterns, false);
        assert_eq!(broker.allowed_patterns.len(), 2);
        assert_eq!(broker.allowed_patterns[0].pattern, "/cmd_vel");
        assert!(broker.allowed_patterns[0].write);
        assert!(!broker.allowed_patterns[1].write);
    }

    // -------------------------------------------------------------------
    // check_access() tests
    // -------------------------------------------------------------------

    #[test]
    fn test_ros_check_access_allowed_exact() {
        let broker = make_broker(vec![rule("/cmd_vel", true, true)], false);
        assert!(broker.check_access("/cmd_vel", false).is_ok());
    }

    #[test]
    fn test_ros_check_access_allowed_glob() {
        let broker = make_broker(vec![rule("/cmd_vel*", true, true)], false);
        assert!(broker.check_access("/cmd_vel", false).is_ok());
        assert!(broker.check_access("/cmd_vel_raw", false).is_ok());
    }

    #[test]
    fn test_ros_check_access_allowed_wildcard() {
        let broker = make_broker(vec![rule("/**", true, true)], false);
        assert!(broker.check_access("/any/topic/at/all", false).is_ok());
        assert!(broker.check_access("/cmd_vel", true).is_ok());
    }

    #[test]
    fn test_ros_check_access_denied_no_match() {
        let broker = make_broker(vec![rule("/cmd_vel", true, true)], false);
        let err = broker.check_access("/scan", false).unwrap_err();
        match err {
            BrokerError::AccessDenied { resource, .. } => {
                assert_eq!(resource, "/scan");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[test]
    fn test_ros_check_access_read_only_blocks_write() {
        let broker = make_broker(vec![rule("/scan", true, false)], false);
        // Read succeeds.
        assert!(broker.check_access("/scan", false).is_ok());
        // Write fails.
        let err = broker.check_access("/scan", true).unwrap_err();
        match err {
            BrokerError::AccessDenied { reason, .. } => {
                assert!(reason.contains("write not permitted"), "reason: {reason}");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[test]
    fn test_ros_check_access_read_write_allows_both() {
        let broker = make_broker(vec![rule("/cmd_vel", true, true)], false);
        assert!(broker.check_access("/cmd_vel", false).is_ok());
        assert!(broker.check_access("/cmd_vel", true).is_ok());
    }

    #[test]
    fn test_ros_check_access_empty_patterns() {
        let broker = make_broker(vec![], false);
        let err = broker.check_access("/anything", false).unwrap_err();
        match err {
            BrokerError::AccessDenied { resource, .. } => {
                assert_eq!(resource, "/anything");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[test]
    fn test_ros_check_access_first_match_wins() {
        // First rule is read-only, second is read-write for the same pattern.
        // Glob matching stops at the first match, so write is denied.
        let broker = make_broker(
            vec![rule("/scan", true, false), rule("/scan", true, true)],
            false,
        );
        assert!(broker.check_access("/scan", true).is_err());
    }

    // -------------------------------------------------------------------
    // handle_request() additional tests
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_ros_topic_subscribe_no_ros2() {
        let broker = make_broker(vec![rule("/cmd_vel", true, true)], false);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::TopicSubscribe {
                topic: "/cmd_vel".into(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::Unavailable { reason, .. } => {
                assert!(reason.contains("ros2 CLI"), "reason: {reason}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_ros_topic_subscribe_access_denied() {
        let broker = make_broker(vec![rule("/cmd_vel", true, true)], true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::TopicSubscribe {
                topic: "/secret_topic".into(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::AccessDenied { resource, .. } => {
                assert_eq!(resource, "/secret_topic");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_ros_unsupported_operation() {
        let broker = make_broker(vec![rule("/**", true, true)], true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::SystemInfo,
        };
        let err = broker.handle_request(req).await.unwrap_err();
        match err {
            BrokerError::AccessDenied { reason, .. } => {
                assert!(reason.contains("not supported"), "reason: {reason}");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_ros_list_succeeds_without_ros2() {
        // RosList does not gate on ros2_available; it always attempts the
        // ros2 CLI and returns whatever it gets (filtered by allowed patterns).
        let broker = make_broker(vec![rule("/**", true, false)], false);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::RosList,
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        let listing: RosListing = serde_json::from_slice(&resp.data).unwrap();
        // On machines without ROS 2, lists are empty but structurally valid.
        let _ = listing.topics;
        let _ = listing.services;
        let _ = listing.nodes;
    }

    #[test]
    fn test_ros_list_filters_by_allowed_patterns() {
        // filter_by_allowed_patterns should keep only names matching at
        // least one allowed pattern and drop the rest.
        let broker = make_broker(
            vec![rule("/cmd_vel", true, false), rule("/scan/**", true, false)],
            false,
        );

        let input = vec![
            "/cmd_vel".to_string(),
            "/scan/front".to_string(),
            "/scan/rear".to_string(),
            "/secret_topic".to_string(),
            "/odom".to_string(),
        ];

        let filtered = broker.filter_by_allowed_patterns(input);
        assert_eq!(filtered, vec!["/cmd_vel", "/scan/front", "/scan/rear"]);
    }

    #[test]
    fn test_ros_list_empty_patterns_filters_everything() {
        let broker = make_broker(vec![], false);
        let input = vec!["/topic_a".to_string(), "/topic_b".to_string()];
        let filtered = broker.filter_by_allowed_patterns(input);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_ros_capability_group() {
        let broker = make_broker(vec![], false);
        assert_eq!(broker.capability_group(), "ganglion:ros/interface");
    }

    // -------------------------------------------------------------------
    // validate_ros_name() tests
    // -------------------------------------------------------------------

    #[test]
    fn test_validate_ros_name_valid() {
        // Standard ROS 2 topic/service/node names.
        assert!(validate_ros_name("/cmd_vel").is_ok());
        assert!(validate_ros_name("/scan/front").is_ok());
        assert!(validate_ros_name("/robot1/joint_states").is_ok());
        assert!(validate_ros_name("/ns/sub-topic").is_ok());
        assert!(validate_ros_name("relative_name").is_ok());
    }

    #[test]
    fn test_validate_ros_name_invalid() {
        // Empty name.
        assert!(validate_ros_name("").is_err());
        // Shell metacharacters.
        assert!(validate_ros_name("/topic;rm -rf /").is_err());
        assert!(validate_ros_name("/topic$(whoami)").is_err());
        assert!(validate_ros_name("/topic`id`").is_err());
        assert!(validate_ros_name("/topic|cat /etc/passwd").is_err());
        assert!(validate_ros_name("/topic&bg").is_err());
        // Control characters.
        assert!(validate_ros_name("/topic\n").is_err());
        assert!(validate_ros_name("/topic\0").is_err());
        // Spaces.
        assert!(validate_ros_name("/topic name").is_err());
        // Dots (not valid in ROS 2 graph names).
        assert!(validate_ros_name("/topic/../etc/passwd").is_err());
    }

    // -------------------------------------------------------------------
    // ros2_command_with_timeout() tests
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_ros2_command_timeout() {
        // We cannot easily test ros2 itself without ROS 2 installed,
        // but we can verify the timeout mechanism works by calling with
        // an effectively-zero timeout.
        let result = ros2_command_with_timeout(&["--version"], Duration::from_millis(1)).await;
        // Either ros2 is not installed (spawn fails) or the timeout
        // fires before it finishes. Both are acceptable error cases.
        assert!(result.is_err());
    }

    #[test]
    fn test_lines_to_vec() {
        let input = "  /topic_a  \n/topic_b\n  \n/topic_c\n";
        let result = lines_to_vec(input);
        assert_eq!(result, vec!["/topic_a", "/topic_b", "/topic_c"]);
    }

    #[test]
    fn test_lines_to_vec_empty() {
        assert!(lines_to_vec("").is_empty());
        assert!(lines_to_vec("  \n  \n").is_empty());
    }
}
