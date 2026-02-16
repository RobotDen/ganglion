use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AuditError;
use crate::identity::PeerId;

/// §3.7: Every capability invocation produces an audit record.
/// Records are written to a local append-only log on the robot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Invoking operator peer ID.
    pub operator_peer_id: PeerId,
    /// Component name.
    pub component_name: String,
    /// Component version.
    pub component_version: String,
    /// Blake3 hash of the component.
    pub component_hash: String,
    /// Capabilities the component used during this invocation.
    pub capabilities_used: Vec<String>,
    /// Wall-clock start time.
    pub started_at: DateTime<Utc>,
    /// Wall-clock end time.
    pub ended_at: DateTime<Utc>,
    /// Exit status.
    pub exit_status: ExitStatus,
    /// Bytes in/out per capability.
    pub io_stats: Vec<CapabilityIoStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    Success,
    Failed { message: String },
    Timeout,
    Trapped { message: String },
    PolicyDenied { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityIoStats {
    pub capability: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// Append-only audit log stored on the robot.
/// Format: newline-delimited CBOR records.
pub struct AuditLog {
    path: PathBuf,
    max_size_bytes: u64,
}

impl AuditLog {
    pub fn new(path: PathBuf, max_size_bytes: u64) -> Self {
        Self {
            path,
            max_size_bytes,
        }
    }

    /// Default log path: /var/lib/gang/audit.log
    pub fn default_path() -> PathBuf {
        PathBuf::from("/var/lib/gang/audit.log")
    }

    /// Append a record to the log.
    pub fn append(&self, record: &AuditRecord) -> Result<(), AuditError> {
        use std::io::Write;

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AuditError::WriteFailed(e.to_string()))?;
        }

        // Check size and rotate if needed
        if let Ok(metadata) = std::fs::metadata(&self.path) {
            if metadata.len() > self.max_size_bytes && self.max_size_bytes > 0 {
                self.rotate()
                    .map_err(|e| AuditError::WriteFailed(e.to_string()))?;
            }
        }

        // Encode record as CBOR
        let mut cbor_bytes = Vec::new();
        ciborium::into_writer(record, &mut cbor_bytes)
            .map_err(|e| AuditError::WriteFailed(e.to_string()))?;

        // Write length-prefixed record
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| AuditError::WriteFailed(e.to_string()))?;

        let len = cbor_bytes.len() as u32;
        file.write_all(&len.to_be_bytes())
            .map_err(|e| AuditError::WriteFailed(e.to_string()))?;
        file.write_all(&cbor_bytes)
            .map_err(|e| AuditError::WriteFailed(e.to_string()))?;

        Ok(())
    }

    /// Read all records from the log.
    pub fn read_all(&self) -> Result<Vec<AuditRecord>, AuditError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let data = std::fs::read(&self.path)
            .map_err(|e| AuditError::Corrupted(e.to_string()))?;

        let mut records = Vec::new();
        let mut offset = 0;

        while offset + 4 <= data.len() {
            let len = u32::from_be_bytes(
                data[offset..offset + 4]
                    .try_into()
                    .map_err(|_| AuditError::Corrupted("invalid length prefix".into()))?,
            ) as usize;
            offset += 4;

            if offset + len > data.len() {
                // Truncated record at end of file — tolerate for crash resilience
                tracing::warn!("truncated audit record at offset {offset}, skipping");
                break;
            }

            let record: AuditRecord = ciborium::from_reader(&data[offset..offset + len])
                .map_err(|e| AuditError::Corrupted(e.to_string()))?;
            records.push(record);
            offset += len;
        }

        Ok(records)
    }

    fn rotate(&self) -> Result<(), std::io::Error> {
        let rotated = self.path.with_extension("log.1");
        // Simple rotation: move current to .1, overwriting any previous .1
        std::fs::rename(&self.path, &rotated)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Keypair;
    use tempfile::TempDir;

    fn sample_record() -> AuditRecord {
        let kp = Keypair::generate();
        AuditRecord {
            operator_peer_id: kp.peer_id(),
            component_name: "diagnostics".into(),
            component_version: "0.1.0".into(),
            component_hash: "abc123".into(),
            capabilities_used: vec!["ganglion:diagnostics/collect@1.0".into()],
            started_at: Utc::now(),
            ended_at: Utc::now(),
            exit_status: ExitStatus::Success,
            io_stats: vec![CapabilityIoStats {
                capability: "ganglion:diagnostics/collect@1.0".into(),
                bytes_in: 0,
                bytes_out: 4096,
            }],
        }
    }

    #[test]
    fn write_and_read_audit_records() {
        let dir = TempDir::new().unwrap();
        let log = AuditLog::new(dir.path().join("audit.log"), 10 * 1024 * 1024);

        let r1 = sample_record();
        let r2 = sample_record();

        log.append(&r1).unwrap();
        log.append(&r2).unwrap();

        let records = log.read_all().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].component_name, "diagnostics");
    }

    #[test]
    fn empty_log_returns_empty() {
        let dir = TempDir::new().unwrap();
        let log = AuditLog::new(dir.path().join("audit.log"), 10 * 1024 * 1024);
        let records = log.read_all().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn log_rotation() {
        let dir = TempDir::new().unwrap();
        // Very small max size to trigger rotation
        let log = AuditLog::new(dir.path().join("audit.log"), 1);

        log.append(&sample_record()).unwrap();
        // Second append should trigger rotation
        log.append(&sample_record()).unwrap();

        assert!(dir.path().join("audit.log").exists());
        assert!(dir.path().join("audit.log.1").exists());
    }
}
