use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Process broker — mediates bounded subprocess invocation from WASM capabilities.
///
/// Enforces:
/// - Command allowlist (only explicitly permitted commands can run)
/// - Wall-clock timeout
/// - Captured stdio (stdout/stderr returned, never inherited)
pub struct ProcessBroker {
    /// Commands that are allowed to execute. Glob patterns supported.
    allowed_commands: Vec<String>,
    /// Maximum wall-clock timeout in seconds. Overrides per-request if lower.
    max_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ProcessBroker {
    pub fn new(allowed_commands: Vec<String>, max_timeout_secs: u64) -> Self {
        Self {
            allowed_commands,
            max_timeout_secs,
        }
    }

    /// Check if a command is on the allowlist.
    fn is_command_allowed(&self, command: &str) -> bool {
        for pattern in &self.allowed_commands {
            if pattern == "**" || pattern == command {
                return true;
            }
            if glob_match::glob_match(pattern, command) {
                return true;
            }
        }
        false
    }

    async fn spawn_process(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &[(String, String)],
        timeout_secs: u64,
    ) -> Result<ProcessOutput, BrokerError> {
        if !self.is_command_allowed(command) {
            return Err(BrokerError::AccessDenied {
                broker: "process".into(),
                resource: command.into(),
                reason: "command not on allowlist".into(),
            });
        }

        let effective_timeout = timeout_secs.min(self.max_timeout_secs);

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        for (key, value) in env {
            cmd.env(key, value);
        }

        let child = cmd.spawn().map_err(|e| BrokerError::Unavailable {
            broker: "process".into(),
            reason: format!("failed to spawn {command}: {e}"),
        })?;

        let result = tokio::time::timeout(
            Duration::from_secs(effective_timeout),
            child.wait_with_output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => Ok(ProcessOutput {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: output.stdout,
                stderr: output.stderr,
            }),
            Ok(Err(e)) => Err(BrokerError::Unavailable {
                broker: "process".into(),
                reason: format!("process error: {e}"),
            }),
            Err(_) => Err(BrokerError::Unavailable {
                broker: "process".into(),
                reason: format!("process timed out after {effective_timeout}s"),
            }),
        }
    }
}

#[async_trait]
impl CapabilityBroker for ProcessBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::ProcessSpawn {
                command,
                args,
                cwd,
                env,
                timeout_secs,
            } => {
                let output = self
                    .spawn_process(&command, &args, cwd.as_deref(), &env, timeout_secs)
                    .await?;

                let data = serde_json::to_vec(&output).map_err(|e| BrokerError::Unavailable {
                    broker: "process".into(),
                    reason: e.to_string(),
                })?;

                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: output.exit_code == 0,
                    data,
                    error: if output.exit_code != 0 {
                        Some(format!("process exited with code {}", output.exit_code))
                    } else {
                        None
                    },
                    bytes_in: 0,
                    bytes_out,
                })
            }
            _ => Err(BrokerError::AccessDenied {
                broker: "process".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by process broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:process/spawn"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_broker() -> ProcessBroker {
        ProcessBroker::new(vec!["echo".into(), "cat".into(), "ls".into()], 10)
    }

    #[test]
    fn command_allowlist_exact_match() {
        let broker = test_broker();
        assert!(broker.is_command_allowed("echo"));
        assert!(broker.is_command_allowed("ls"));
        assert!(!broker.is_command_allowed("rm"));
        assert!(!broker.is_command_allowed("bash"));
    }

    #[test]
    fn command_allowlist_glob() {
        let broker = ProcessBroker::new(vec!["/usr/bin/*".into()], 10);
        assert!(broker.is_command_allowed("/usr/bin/echo"));
        assert!(!broker.is_command_allowed("/usr/sbin/reboot"));
    }

    #[test]
    fn wildcard_allows_everything() {
        let broker = ProcessBroker::new(vec!["**".into()], 10);
        assert!(broker.is_command_allowed("anything"));
    }

    #[tokio::test]
    async fn spawn_echo() {
        let broker = test_broker();
        let output = broker
            .spawn_process("echo", &["hello".into()], None, &[], 5)
            .await
            .unwrap();
        assert_eq!(output.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[tokio::test]
    async fn spawn_denied_command() {
        let broker = test_broker();
        let result = broker
            .spawn_process("rm", &["-rf".into(), "/".into()], None, &[], 5)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            BrokerError::AccessDenied { resource, .. } => {
                assert_eq!(resource, "rm");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn broker_handle_request() {
        let broker = test_broker();
        let req = CapabilityRequest {
            capability_group: "ganglion:process/spawn".into(),
            operation: BrokerOperation::ProcessSpawn {
                command: "echo".into(),
                args: vec!["broker test".into()],
                cwd: None,
                env: vec![],
                timeout_secs: 5,
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        let output: ProcessOutput = serde_json::from_slice(&resp.data).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "broker test"
        );
    }

    #[tokio::test]
    async fn broker_rejects_unsupported_op() {
        let broker = test_broker();
        let req = CapabilityRequest {
            capability_group: "ganglion:process/spawn".into(),
            operation: BrokerOperation::SystemInfo,
        };
        assert!(broker.handle_request(req).await.is_err());
    }
}
