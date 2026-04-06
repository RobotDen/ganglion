use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

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
        let ros2_available = check_ros2_available();

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
                let topics = self.filter_by_allowed_patterns(ros2_topic_list());
                let services = self.filter_by_allowed_patterns(ros2_service_list());
                let nodes = self.filter_by_allowed_patterns(ros2_node_list());

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
                let info = ros2_topic_info(topic);
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

                // TODO: When WebSocket transport to rosbridge is wired up,
                // send a `call_service` rosbridge op with a per-call timeout.
                // Rosbridge protocol expects:
                //   { "op": "call_service", "service": "<name>",
                //     "args": <json>, "id": "<uuid>" }
                // The response arrives asynchronously on the same socket.
                // For now, format the request and return a placeholder response
                // indicating the call was accepted by the broker.
                let bytes_in = request.len() as u64;
                let response = serde_json::json!({
                    "op": "call_service",
                    "service": service,
                    "status": "accepted",
                    "note": "rosbridge transport pending — request formatted but not dispatched",
                });
                let data = serde_json::to_vec(&response).map_err(|e| BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: true,
                    data,
                    error: None,
                    bytes_in,
                    bytes_out,
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

                // In full implementation: query the parameter via rosbridge
                // using the `get_param` op or ros2 CLI fallback.
                // For now, return a structured response confirming the
                // broker accepted the request and the parameter name is valid.
                let response = serde_json::json!({
                    "op": "get_param",
                    "name": name,
                    "status": "accepted",
                    "note": "rosbridge transport pending — request formatted but not dispatched",
                });
                let data = serde_json::to_vec(&response).map_err(|e| BrokerError::Unavailable {
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

                // In full implementation: set the parameter via rosbridge
                // using the `set_param` op or ros2 CLI fallback.
                // For now, return a structured response confirming the
                // broker accepted the request.
                let bytes_in = value.len() as u64;
                let response = serde_json::json!({
                    "op": "set_param",
                    "name": name,
                    "value_size": value.len(),
                    "status": "accepted",
                    "note": "rosbridge transport pending — request formatted but not dispatched",
                });
                let data = serde_json::to_vec(&response).map_err(|e| BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: true,
                    data,
                    error: None,
                    bytes_in,
                    bytes_out,
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

/// Check whether the `ros2` CLI tools are accessible, which is a proxy for
/// whether ROS 2 is installed and the environment is sourced.
///
/// NOTE: When WebSocket rosbridge transport is added, this should check
/// port 9090 connectivity instead (or in addition).
fn check_ros2_available() -> bool {
    std::process::Command::new("ros2")
        .args(["topic", "list"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// List all ROS 2 topics by shelling out to `ros2 topic list`.
///
/// Uses the ros2 CLI directly. This is intentional: the broker runs as a
/// privileged Layer 3 host process. Rosbridge WebSocket transport is planned
/// for future versions.
fn ros2_topic_list() -> Vec<String> {
    std::process::Command::new("ros2")
        .args(["topic", "list"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// List all ROS 2 services by shelling out to `ros2 service list`.
///
/// Uses the ros2 CLI directly. Rosbridge WebSocket transport is planned
/// for future versions.
fn ros2_service_list() -> Vec<String> {
    std::process::Command::new("ros2")
        .args(["service", "list"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// List all ROS 2 nodes by shelling out to `ros2 node list`.
///
/// Uses the ros2 CLI directly. Rosbridge WebSocket transport is planned
/// for future versions.
fn ros2_node_list() -> Vec<String> {
    std::process::Command::new("ros2")
        .args(["node", "list"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Get verbose info for a single ROS 2 topic by shelling out to
/// `ros2 topic info <topic> -v`.
///
/// Uses the ros2 CLI directly. Rosbridge WebSocket transport is planned
/// for future versions.
fn ros2_topic_info(topic: &str) -> serde_json::Value {
    let output = std::process::Command::new("ros2")
        .args(["topic", "info", topic, "-v"])
        .output();

    match output {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            serde_json::json!({
                "topic": topic,
                "info": text,
                "available": o.status.success(),
            })
        }
        Err(e) => serde_json::json!({
            "topic": topic,
            "error": e.to_string(),
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
    async fn param_set_succeeds_with_write_access() {
        let broker = rw_broker(true);
        let req = param_set_request("/params/max_speed", vec![0x42, 0x28, 0x00, 0x00]);
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(resp.bytes_in, 4);
        assert!(resp.bytes_out > 0);

        let body: serde_json::Value = serde_json::from_slice(&resp.data).unwrap();
        assert_eq!(body["name"], "/params/max_speed");
        assert_eq!(body["status"], "accepted");
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
    async fn param_get_returns_data_when_ros2_available() {
        let broker = rw_broker(true);
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ParamGet {
                name: "/params/max_speed".into(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert!(resp.bytes_out > 0);
        assert_eq!(resp.bytes_in, 0);

        let value: serde_json::Value = serde_json::from_slice(&resp.data).unwrap();
        assert_eq!(value["op"], "get_param");
        assert_eq!(value["name"], "/params/max_speed");
        assert_eq!(value["status"], "accepted");
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
    async fn service_call_returns_data_when_ros2_available() {
        let broker = rw_broker(true);
        let request_payload = b"{}".to_vec();
        let req = CapabilityRequest {
            capability_group: "ganglion:ros/interface".into(),
            operation: BrokerOperation::ServiceCall {
                service: "/params/set_bool".into(),
                request: request_payload.clone(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(resp.bytes_in, request_payload.len() as u64);
        assert!(resp.bytes_out > 0);

        let value: serde_json::Value = serde_json::from_slice(&resp.data).unwrap();
        assert_eq!(value["op"], "call_service");
        assert_eq!(value["service"], "/params/set_bool");
        assert_eq!(value["status"], "accepted");
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
}
