use std::io::{Seek, SeekFrom};

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
                // Source enumeration execs `journalctl --version` and stats
                // paths — blocking work that must not run inline on the
                // async executor.
                let sources = tokio::task::spawn_blocking(enumerate_log_sources)
                    .await
                    .map_err(|e| BrokerError::Unavailable {
                        broker: "logs".into(),
                        reason: format!("log source enumeration task failed: {e}"),
                    })?;
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

                // CODE-10: the journald subprocess and file tail-read are
                // blocking — run them off the async executor.
                let source = source.clone();
                let pattern = pattern.clone();
                let lines = tokio::task::spawn_blocking(move || read_log_source(&source, &pattern))
                    .await
                    .map_err(|e| BrokerError::Unavailable {
                        broker: "logs".into(),
                        reason: format!("log read task failed: {e}"),
                    })?;
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

/// Number of trailing lines returned by a log read.
const TAIL_LINES: usize = 100;

/// Maximum bytes read from the end of a log file. Bounds memory regardless of
/// file size (CODE-10) — we never read the whole file.
const MAX_TAIL_BYTES: u64 = 1024 * 1024;

/// Read the tail of the journal.
///
/// Lines are returned in chronological order (oldest first), matching
/// `journalctl`'s natural `-n` output and the file reader below.
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

/// Read the last [`TAIL_LINES`] lines of a log file without loading the whole
/// file into memory (CODE-10): seek to at most [`MAX_TAIL_BYTES`] before the
/// end and read forward from there.
///
/// Lines are returned in chronological order (oldest first), consistent with
/// [`read_journald`].
fn read_log_file(path: &str, pattern: &str) -> Vec<LogLine> {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    let read_len = len.min(MAX_TAIL_BYTES);
    let start = len - read_len;

    // When the window starts mid-file, also read the single byte before it:
    // if that byte is '\n' the window begins exactly at a line boundary and
    // its first line is complete; otherwise the first line is a truncated
    // fragment that must be dropped. (Unconditionally dropping the first
    // line discarded a complete line at exact boundaries — off-by-one.)
    let peek = u64::from(start > 0);
    if file.seek(SeekFrom::Start(start - peek)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::with_capacity((read_len + peek) as usize);
    if file.take(read_len + peek).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let first_is_fragment = peek == 1 && buf.first() != Some(&b'\n');
    let window = &buf[peek as usize..];

    let text = String::from_utf8_lossy(window);
    let mut lines: Vec<&str> = text.lines().collect();
    if first_is_fragment && !lines.is_empty() {
        lines.remove(0);
    }
    // Keep the last TAIL_LINES, preserving chronological (oldest-first) order.
    let tail_start = lines.len().saturating_sub(TAIL_LINES);
    lines[tail_start..]
        .iter()
        .filter(|line| pattern.is_empty() || line.contains(pattern))
        .map(|line| LogLine {
            timestamp: "".into(),
            source: path.into(),
            level: "info".into(),
            message: (*line).into(),
        })
        .collect()
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

    #[test]
    fn read_log_file_tails_in_chronological_order() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.log");
        let mut f = std::fs::File::create(&path).unwrap();
        // Write 500 numbered lines; only the last 100 should come back.
        for i in 0..500 {
            writeln!(f, "line {i}").unwrap();
        }
        f.flush().unwrap();

        let lines = read_log_file(path.to_str().unwrap(), "");
        assert_eq!(lines.len(), TAIL_LINES);
        // Oldest-first: first returned line is 400, last is 499.
        assert_eq!(lines.first().unwrap().message, "line 400");
        assert_eq!(lines.last().unwrap().message, "line 499");
    }

    #[test]
    fn read_log_file_window_at_exact_line_boundary_keeps_first_line() {
        // 16 KiB lines: the 1 MiB window holds exactly 64 of them. With 80
        // lines in the file, the window starts exactly at the '\n' boundary
        // before line 16 — the first windowed line is COMPLETE and must not
        // be dropped (regression: it was discarded unconditionally).
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("boundary.log");
        let mut f = std::fs::File::create(&path).unwrap();
        let line_len = 16 * 1024; // including the trailing '\n'
        for i in 0..80 {
            let head = format!("line {i:05} ");
            let pad = "x".repeat(line_len - 1 - head.len());
            writeln!(f, "{head}{pad}").unwrap();
        }
        f.flush().unwrap();
        // Sanity: window (1 MiB = 64 lines) starts exactly at a line boundary.
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            80 * line_len as u64
        );
        assert_eq!(MAX_TAIL_BYTES % line_len as u64, 0);

        let lines = read_log_file(path.to_str().unwrap(), "");
        assert_eq!(
            lines.len(),
            64,
            "window holds 64 complete lines; none may be dropped"
        );
        assert!(lines.first().unwrap().message.starts_with("line 00016"));
        assert!(lines.last().unwrap().message.starts_with("line 00079"));
    }

    #[test]
    fn read_log_file_window_mid_line_drops_fragment() {
        // An unterminated 7-byte prefix shifts the window so it starts
        // mid-line; the leading fragment must still be dropped.
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("midline.log");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "prefix:").unwrap(); // no newline — merges with line 0
        let line_len = 16 * 1024;
        for i in 0..64 {
            let head = format!("line {i:05} ");
            let pad = "x".repeat(line_len - 1 - head.len());
            writeln!(f, "{head}{pad}").unwrap();
        }
        f.flush().unwrap();

        let lines = read_log_file(path.to_str().unwrap(), "");
        // The fragment of the merged first line is dropped; 63 complete
        // lines remain, starting at line 1.
        assert_eq!(lines.len(), 63);
        assert!(lines.first().unwrap().message.starts_with("line 00001"));
        assert!(lines.last().unwrap().message.starts_with("line 00063"));
    }

    #[test]
    fn read_log_file_filters_by_pattern() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("f.log");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "info: all good").unwrap();
        writeln!(f, "ERROR: boom").unwrap();
        writeln!(f, "info: fine").unwrap();
        f.flush().unwrap();

        let lines = read_log_file(path.to_str().unwrap(), "ERROR");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].message, "ERROR: boom");
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
