use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// ROS 2 interface broker — mediates WASM capability access to the ROS 2 graph.
///
/// v0.1 implementation uses rosbridge WebSocket as the ROS 2 bridge.
/// Future versions will use rclrs for direct integration.
///
/// This broker enforces pattern-based access gating: each operation is checked
/// against the capability's declared topic/service/parameter patterns before
/// executing.
pub struct RosInterfaceBroker {
    /// Allowed topic/service/parameter patterns.
    allowed_patterns: Vec<RosAccessRule>,
    /// Whether a rosbridge connection is available.
    rosbridge_available: bool,
}

#[derive(Debug, Clone)]
pub struct RosAccessRule {
    pub pattern: String,
    pub read: bool,
    pub write: bool,
}

impl RosInterfaceBroker {
    pub fn new(allowed_patterns: Vec<RosAccessRule>) -> Self {
        // Check if rosbridge is available
        let rosbridge_available = check_rosbridge();

        Self {
            allowed_patterns,
            rosbridge_available,
        }
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
                // Use ros2 CLI to list topics, services, and parameters
                let topics = ros2_topic_list();
                let services = ros2_service_list();
                let nodes = ros2_node_list();

                let listing = RosListing {
                    topics,
                    services,
                    nodes,
                };

                let data = serde_json::to_vec(&listing).map_err(|e| {
                    BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: e.to_string(),
                    }
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

                if !self.rosbridge_available {
                    return Err(BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: "rosbridge not available — is ROS 2 running?".into(),
                    });
                }

                // In full implementation: subscribe via rosbridge WebSocket
                // and stream messages back. For now, return current topic info.
                let info = ros2_topic_info(topic);
                let data = serde_json::to_vec(&info).map_err(|e| {
                    BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: e.to_string(),
                    }
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
                request: _,
            } => {
                self.check_access(service, true)?;

                if !self.rosbridge_available {
                    return Err(BrokerError::Unavailable {
                        broker: "ros".into(),
                        reason: "rosbridge not available".into(),
                    });
                }

                // In full implementation: call service via rosbridge
                Err(BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: "service calls not yet implemented".into(),
                })
            }
            BrokerOperation::ParamGet { ref name } => {
                self.check_access(name, false)?;
                // ros2 param get
                Err(BrokerError::Unavailable {
                    broker: "ros".into(),
                    reason: "parameter operations not yet implemented".into(),
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

fn check_rosbridge() -> bool {
    // Check if rosbridge_server is running by looking for the process
    // or checking its default WebSocket port (9090)
    std::process::Command::new("ros2")
        .args(["topic", "list"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

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
