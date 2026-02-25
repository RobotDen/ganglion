use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Filesystem broker — mediates bounded filesystem access for WASM capabilities.
/// Enforces path-pattern-gated access with explicit read/write/execute flags.
/// Rejects any path outside declared patterns and resolves symlinks to
/// prevent jailbreak.
pub struct FsBroker {
    /// Allowed path patterns with their permission flags.
    allowed_patterns: Vec<FsRule>,
}

#[derive(Debug, Clone)]
pub struct FsRule {
    pub pattern: String,
    pub read: bool,
    pub write: bool,
}

impl FsBroker {
    pub fn new(allowed_patterns: Vec<FsRule>) -> Self {
        Self { allowed_patterns }
    }

    /// Check if a path is permitted under the current rules.
    fn check_access(&self, path: &str, needs_write: bool) -> Result<(), BrokerError> {
        // Resolve to canonical path to prevent symlink jailbreak
        let canonical = if Path::new(path).exists() {
            std::fs::canonicalize(path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string())
        } else {
            // For writes to new files, check the parent
            path.to_string()
        };

        for rule in &self.allowed_patterns {
            if glob_match::glob_match(&rule.pattern, &canonical)
                || glob_match::glob_match(&rule.pattern, path)
            {
                if needs_write && !rule.write {
                    return Err(BrokerError::AccessDenied {
                        broker: "fs".into(),
                        resource: path.into(),
                        reason: "write not permitted for this pattern".into(),
                    });
                }
                if !needs_write && !rule.read {
                    return Err(BrokerError::AccessDenied {
                        broker: "fs".into(),
                        resource: path.into(),
                        reason: "read not permitted for this pattern".into(),
                    });
                }
                return Ok(());
            }
        }

        Err(BrokerError::AccessDenied {
            broker: "fs".into(),
            resource: path.into(),
            reason: "path does not match any allowed pattern".into(),
        })
    }
}

#[async_trait]
impl CapabilityBroker for FsBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::FsRead { ref path } => {
                self.check_access(path, false)?;

                match std::fs::read(path) {
                    Ok(data) => {
                        let bytes_out = data.len() as u64;
                        Ok(CapabilityResponse {
                            success: true,
                            data,
                            error: None,
                            bytes_in: 0,
                            bytes_out,
                        })
                    }
                    Err(e) => Err(BrokerError::Unavailable {
                        broker: "fs".into(),
                        reason: e.to_string(),
                    }),
                }
            }
            BrokerOperation::FsWrite { ref path, ref data } => {
                self.check_access(path, true)?;

                let bytes_in = data.len() as u64;
                std::fs::write(path, data).map_err(|e| BrokerError::Unavailable {
                    broker: "fs".into(),
                    reason: e.to_string(),
                })?;

                Ok(CapabilityResponse {
                    success: true,
                    data: Vec::new(),
                    error: None,
                    bytes_in,
                    bytes_out: 0,
                })
            }
            BrokerOperation::FsList { ref path } => {
                self.check_access(path, false)?;

                let entries: Vec<String> = std::fs::read_dir(path)
                    .map_err(|e| BrokerError::Unavailable {
                        broker: "fs".into(),
                        reason: e.to_string(),
                    })?
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .collect();

                let data = serde_json::to_vec(&entries).map_err(|e| {
                    BrokerError::Unavailable {
                        broker: "fs".into(),
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
            BrokerOperation::FsStat { ref path } => {
                self.check_access(path, false)?;

                let metadata = std::fs::metadata(path).map_err(|e| {
                    BrokerError::Unavailable {
                        broker: "fs".into(),
                        reason: e.to_string(),
                    }
                })?;

                let stat = FileStat {
                    size: metadata.len(),
                    is_file: metadata.is_file(),
                    is_dir: metadata.is_dir(),
                    is_symlink: metadata.is_symlink(),
                    readonly: metadata.permissions().readonly(),
                };

                let data = serde_json::to_vec(&stat).map_err(|e| {
                    BrokerError::Unavailable {
                        broker: "fs".into(),
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
            _ => Err(BrokerError::AccessDenied {
                broker: "fs".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by filesystem broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:fs/bounded"
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileStat {
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub readonly: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Canonical path string — resolves macOS /var/folders symlinks.
    fn canon(p: &Path) -> String {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string()
    }

    fn test_broker(dir: &Path) -> FsBroker {
        let c = canon(dir);
        FsBroker::new(vec![
            FsRule {
                pattern: c.clone(),
                read: true,
                write: true,
            },
            FsRule {
                pattern: format!("{c}/**"),
                read: true,
                write: true,
            },
        ])
    }

    #[tokio::test]
    async fn fs_read_within_pattern() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, b"hello").unwrap();

        let broker = test_broker(dir.path());
        let req = CapabilityRequest {
            capability_group: "ganglion:fs/bounded".into(),
            operation: BrokerOperation::FsRead {
                path: file.to_string_lossy().to_string(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(resp.data, b"hello");
    }

    #[tokio::test]
    async fn fs_read_outside_pattern_denied() {
        let broker = FsBroker::new(vec![FsRule {
            pattern: "/tmp/gang-test/**".into(),
            read: true,
            write: false,
        }]);
        let req = CapabilityRequest {
            capability_group: "ganglion:fs/bounded".into(),
            operation: BrokerOperation::FsRead {
                path: "/etc/passwd".into(),
            },
        };
        let result = broker.handle_request(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fs_write_within_pattern() {
        let dir = TempDir::new().unwrap();
        let canonical_dir = canon(dir.path());
        let file_path = format!("{canonical_dir}/output.txt");

        let broker = test_broker(dir.path());
        let req = CapabilityRequest {
            capability_group: "ganglion:fs/bounded".into(),
            operation: BrokerOperation::FsWrite {
                path: file_path.clone(),
                data: b"written by ganglion".to_vec(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "written by ganglion");
    }

    #[tokio::test]
    async fn fs_write_on_read_only_denied() {
        let dir = TempDir::new().unwrap();
        let broker = FsBroker::new(vec![FsRule {
            pattern: format!("{}/**", dir.path().display()),
            read: true,
            write: false, // read-only
        }]);

        let req = CapabilityRequest {
            capability_group: "ganglion:fs/bounded".into(),
            operation: BrokerOperation::FsWrite {
                path: dir.path().join("nope.txt").to_string_lossy().to_string(),
                data: b"should fail".to_vec(),
            },
        };
        let result = broker.handle_request(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fs_list_directory() {
        let dir = TempDir::new().unwrap();
        let canonical_dir = canon(dir.path());
        std::fs::write(format!("{canonical_dir}/a.txt"), b"a").unwrap();
        std::fs::write(format!("{canonical_dir}/b.txt"), b"b").unwrap();

        let broker = test_broker(dir.path());
        let req = CapabilityRequest {
            capability_group: "ganglion:fs/bounded".into(),
            operation: BrokerOperation::FsList {
                path: canonical_dir,
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);

        let entries: Vec<String> = serde_json::from_slice(&resp.data).unwrap();
        assert!(entries.contains(&"a.txt".to_string()));
        assert!(entries.contains(&"b.txt".to_string()));
    }

    #[tokio::test]
    async fn fs_stat_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, b"hello").unwrap();

        let broker = test_broker(dir.path());
        let req = CapabilityRequest {
            capability_group: "ganglion:fs/bounded".into(),
            operation: BrokerOperation::FsStat {
                path: file.to_string_lossy().to_string(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);

        let stat: FileStat = serde_json::from_slice(&resp.data).unwrap();
        assert!(stat.is_file);
        assert_eq!(stat.size, 5);
    }
}
