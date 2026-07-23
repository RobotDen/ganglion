use std::path::Path;
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
    /// Patterns are matched against the absolute, canonicalized command path.
    allowed_commands: Vec<String>,
    /// Maximum wall-clock timeout in seconds. Overrides per-request if lower.
    max_timeout_secs: u64,
}

/// Safe default `PATH` installed for every spawned child. The environment is
/// scrubbed before spawn, so this is the only `PATH` a child ever sees.
const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

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
        // Reject relative commands: a relative path would be resolved via
        // PATH lookup or against an attacker-controlled cwd, letting a
        // caller run an arbitrary binary that merely shares a name with an
        // allowlisted one. Require an absolute path.
        if !Path::new(command).is_absolute() {
            return Err(BrokerError::AccessDenied {
                broker: "process".into(),
                resource: command.into(),
                reason: "command must be an absolute path".into(),
            });
        }

        // Canonicalize (resolve symlinks and `..`) so the allowlist is matched
        // against the real executable, not an alias or traversal that points
        // at a different binary.
        let canonical = std::fs::canonicalize(command).map_err(|e| BrokerError::AccessDenied {
            broker: "process".into(),
            resource: command.into(),
            reason: format!("cannot resolve command path: {e}"),
        })?;
        let canonical_str = canonical.to_string_lossy().to_string();

        if !self.is_command_allowed(&canonical_str) {
            return Err(BrokerError::AccessDenied {
                broker: "process".into(),
                resource: canonical_str,
                reason: "command not on allowlist".into(),
            });
        }

        let effective_timeout = timeout_secs.min(self.max_timeout_secs);

        let mut cmd = tokio::process::Command::new(&canonical);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            // CODE-04: ensure a timed-out (or otherwise dropped) child is
            // reaped instead of outliving the reported timeout.
            .kill_on_drop(true);

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        // Scrub the environment: start from empty, install a safe PATH, and
        // drop dynamic-linker hijack vectors (LD_PRELOAD, LD_LIBRARY_PATH,
        // etc.) as well as any caller-supplied PATH override.
        cmd.env_clear();
        cmd.env("PATH", SAFE_PATH);
        for (key, value) in env {
            if key.starts_with("LD_") || key.eq_ignore_ascii_case("PATH") {
                continue;
            }
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

    /// Resolve an absolute, canonical path for a common binary, skipping the
    /// test if it is not present on this host.
    fn locate(bin: &str) -> Option<String> {
        for dir in ["/usr/bin", "/bin", "/usr/local/bin"] {
            let p = std::path::Path::new(dir).join(bin);
            if p.exists() {
                if let Ok(c) = std::fs::canonicalize(&p) {
                    return Some(c.to_string_lossy().to_string());
                }
            }
        }
        None
    }

    /// Broker that allows any absolute command under the standard bin dirs.
    fn test_broker() -> ProcessBroker {
        ProcessBroker::new(
            vec![
                "/usr/bin/*".into(),
                "/bin/*".into(),
                "/usr/local/bin/*".into(),
            ],
            10,
        )
    }

    #[test]
    fn command_allowlist_exact_match() {
        let broker = ProcessBroker::new(vec!["/usr/bin/echo".into(), "/usr/bin/ls".into()], 10);
        assert!(broker.is_command_allowed("/usr/bin/echo"));
        assert!(broker.is_command_allowed("/usr/bin/ls"));
        assert!(!broker.is_command_allowed("/usr/bin/rm"));
        assert!(!broker.is_command_allowed("/bin/bash"));
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
        let Some(echo) = locate("echo") else { return };
        let broker = test_broker();
        let output = broker
            .spawn_process(&echo, &["hello".into()], None, &[], 5)
            .await
            .unwrap();
        assert_eq!(output.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[tokio::test]
    async fn spawn_relative_command_rejected() {
        // SEC-11: a relative command must be rejected outright, even if a
        // binary of that name exists on PATH and would match the allowlist
        // after resolution.
        let broker = ProcessBroker::new(vec!["**".into()], 10);
        let err = broker
            .spawn_process("echo", &["hi".into()], None, &[], 5)
            .await
            .unwrap_err();
        match err {
            BrokerError::AccessDenied {
                resource, reason, ..
            } => {
                assert_eq!(resource, "echo");
                assert!(reason.contains("absolute"), "reason: {reason}");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn spawn_ld_preload_stripped() {
        // SEC-11: LD_* variables must never reach the child. `env` prints the
        // scrubbed environment; LD_PRELOAD must be absent and PATH must be the
        // safe default.
        let Some(env_bin) = locate("env") else { return };
        let broker = test_broker();
        let output = broker
            .spawn_process(
                &env_bin,
                &[],
                None,
                &[
                    ("LD_PRELOAD".into(), "/tmp/evil.so".into()),
                    ("LD_LIBRARY_PATH".into(), "/tmp".into()),
                    ("PATH".into(), "/tmp/attacker".into()),
                    ("SAFE_VAR".into(), "kept".into()),
                ],
                5,
            )
            .await
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("LD_PRELOAD"),
            "LD_PRELOAD leaked: {stdout}"
        );
        assert!(
            !stdout.contains("LD_LIBRARY_PATH"),
            "LD_LIBRARY_PATH leaked: {stdout}"
        );
        assert!(!stdout.contains("/tmp/attacker"), "PATH override leaked");
        assert!(
            stdout.contains(&format!("PATH={SAFE_PATH}")),
            "safe PATH missing"
        );
        assert!(stdout.contains("SAFE_VAR=kept"), "benign var dropped");
    }

    #[tokio::test]
    async fn spawn_denied_command() {
        // An absolute command outside the allowlist is denied.
        let broker = ProcessBroker::new(vec!["/usr/bin/echo".into()], 10);
        let Some(rm) = locate("rm").or_else(|| Some("/usr/bin/rm".into())) else {
            return;
        };
        let result = broker
            .spawn_process(&rm, &["-rf".into(), "/".into()], None, &[], 5)
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            BrokerError::AccessDenied { .. }
        ));
    }

    #[tokio::test]
    async fn spawn_timeout_kills_child() {
        // CODE-04: a command exceeding the timeout returns an error and the
        // child is killed (kill_on_drop), not left running past the timeout.
        let Some(sleep) = locate("sleep") else { return };
        let broker = test_broker();
        let start = std::time::Instant::now();
        let result = broker
            .spawn_process(&sleep, &["30".into()], None, &[], 1)
            .await;
        assert!(result.is_err(), "sleep 30 with 1s timeout must error");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timeout must fire promptly, not wait for the child"
        );
    }

    #[tokio::test]
    async fn broker_handle_request() {
        let Some(echo) = locate("echo") else { return };
        let broker = test_broker();
        let req = CapabilityRequest {
            capability_group: "ganglion:process/spawn".into(),
            operation: BrokerOperation::ProcessSpawn {
                command: echo,
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
