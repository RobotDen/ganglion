use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Log stream broker — provides read access to system and ROS logs.
/// Gated by log source patterns from the capability manifest.
pub struct LogStreamBroker {
    allowed_sources: Vec<String>,
}

impl LogStreamBroker {
    pub fn new(allowed_sources: Vec<String>) -> Self {
        Self { allowed_sources }
    }

    /// Permissive broker that allows all log sources.
    pub fn permissive() -> Self {
        Self {
            allowed_sources: vec!["**".into()],
        }
    }

    fn check_source_allowed(&self, source: &str) -> Result<(), BrokerError> {
        for pattern in &self.allowed_sources {
            if pattern == "**" || glob_match::glob_match(pattern, source) {
                return Ok(());
            }
        }
        Err(BrokerError::AccessDenied {
            broker: "logs".into(),
            resource: source.into(),
            reason: "log source not permitted".into(),
        })
    }
}

#[async_trait]
impl CapabilityBroker for LogStreamBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::LogSourceList => {
                let sources = enumerate_log_sources();
                let data = serde_json::to_vec(&sources).map_err(|e| BrokerError::Unavailable {
                    broker: "logs".into(),
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
            BrokerOperation::LogStream {
                ref source,
                ref pattern,
            } => {
                self.check_source_allowed(source)?;

                let lines = read_log_source(source, pattern);
                let data = serde_json::to_vec(&lines).map_err(|e| BrokerError::Unavailable {
                    broker: "logs".into(),
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
            _ => Err(BrokerError::AccessDenied {
                broker: "logs".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by log stream broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:logs/stream"
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogSource {
    pub name: String,
    pub source_type: LogSourceType,
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogSourceType {
    Journald,
    File,
    RosTopic,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogLine {
    pub timestamp: String,
    pub source: String,
    pub level: String,
    pub message: String,
}

fn enumerate_log_sources() -> Vec<LogSource> {
    let mut sources = Vec::new();

    // Check for journald
    if std::process::Command::new("journalctl")
        .args(["--version"])
        .output()
        .is_ok()
    {
        sources.push(LogSource {
            name: "journald".into(),
            source_type: LogSourceType::Journald,
            available: true,
        });
    }

    // Check for common log files
    for path in &[
        "/var/log/syslog",
        "/var/log/messages",
        "/var/log/system.log",
    ] {
        if std::path::Path::new(path).exists() {
            sources.push(LogSource {
                name: path.to_string(),
                source_type: LogSourceType::File,
                available: true,
            });
        }
    }

    // Check for ROS log directory
    if let Ok(home) = std::env::var("HOME") {
        let ros_log_dir = format!("{home}/.ros/log");
        if std::path::Path::new(&ros_log_dir).exists() {
            sources.push(LogSource {
                name: "ros_log_dir".into(),
                source_type: LogSourceType::File,
                available: true,
            });
        }
    }

    sources
}

fn read_log_source(source: &str, pattern: &str) -> Vec<LogLine> {
    match source {
        "journald" => read_journald(pattern),
        _ if source.starts_with('/') => read_log_file(source, pattern),
        _ => Vec::new(),
    }
}

fn read_journald(pattern: &str) -> Vec<LogLine> {
    let output = std::process::Command::new("journalctl")
        .args(["--no-pager", "-n", "100", "--output=short-iso"])
        .output();

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .filter(|line| pattern.is_empty() || line.contains(pattern))
                .map(|line| LogLine {
                    timestamp: line.split_whitespace().next().unwrap_or("").into(),
                    source: "journald".into(),
                    level: "info".into(),
                    message: line.into(),
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

fn read_log_file(path: &str, pattern: &str) -> Vec<LogLine> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            // Read last 100 lines
            content
                .lines()
                .rev()
                .take(100)
                .filter(|line| pattern.is_empty() || line.contains(pattern))
                .map(|line| LogLine {
                    timestamp: "".into(),
                    source: path.into(),
                    level: "info".into(),
                    message: line.into(),
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_sources() {
        let sources = enumerate_log_sources();
        // Should find at least something on any system
        // (may be empty in minimal containers)
        let _ = sources;
    }

    #[tokio::test]
    async fn log_broker_list_sources() {
        let broker = LogStreamBroker::permissive();
        let req = CapabilityRequest {
            capability_group: "ganglion:logs/stream".into(),
            operation: BrokerOperation::LogSourceList,
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
    }

    #[tokio::test]
    async fn log_broker_denies_unmatched_source() {
        let broker = LogStreamBroker::new(vec!["journald".into()]);
        let req = CapabilityRequest {
            capability_group: "ganglion:logs/stream".into(),
            operation: BrokerOperation::LogStream {
                source: "/var/log/syslog".into(),
                pattern: "".into(),
            },
        };
        let result = broker.handle_request(req).await;
        assert!(result.is_err());
    }
}
