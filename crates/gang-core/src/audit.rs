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

/// Terminal status of an audited capability invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    /// The invocation completed successfully.
    Success,
    /// The invocation failed with a message.
    Failed {
        /// Failure message.
        message: String,
    },
    /// The invocation exceeded its deadline.
    Timeout,
    /// The WASM guest trapped.
    Trapped {
        /// Trap message.
        message: String,
    },
    /// The invocation was denied by policy.
    PolicyDenied {
        /// Reason for denial.
        reason: String,
    },
}

/// Per-capability byte counters recorded for an invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityIoStats {
    /// The capability these counters apply to.
    pub capability: String,
    /// Bytes read from the resource.
    pub bytes_in: u64,
    /// Bytes written to the resource.
    pub bytes_out: u64,
}

/// On-disk representation of an audit record together with its hash-chain
/// metadata. The chain fields default when absent, so pre-chain (legacy)
/// records still deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChainedRecord {
    #[serde(flatten)]
    record: AuditRecord,
    /// Monotonic position in the chain (0-based).
    #[serde(default)]
    seq: u64,
    /// Hex hash of the previous record; empty for the genesis record.
    #[serde(default)]
    prev_hash: String,
    /// Hex `blake3(prev_hash || seq_be || cbor(record))`. Empty for legacy
    /// records written before the chain existed.
    #[serde(default)]
    record_hash: String,
}

impl ChainedRecord {
    /// Compute the chain hash for a record given its predecessor hash and seq.
    fn compute_hash(record: &AuditRecord, seq: u64, prev_hash: &str) -> Result<String, AuditError> {
        let mut record_cbor = Vec::new();
        ciborium::into_writer(record, &mut record_cbor)
            .map_err(|e| AuditError::WriteFailed(e.to_string()))?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(prev_hash.as_bytes());
        hasher.update(&seq.to_be_bytes());
        hasher.update(&record_cbor);
        Ok(hasher.finalize().to_hex().to_string())
    }
}

/// Append-only audit log stored on the robot.
///
/// Format: a sequence of length-prefixed CBOR `ChainedRecord`s (an internal
/// envelope pairing each [`AuditRecord`] with its chain metadata). Each record
/// embeds a Blake3 hash chain (`blake3(prev_hash || seq || cbor(record))`) so
/// tampering, reordering, and interior deletion are detectable via
/// [`AuditLog::verify_chain`].
pub struct AuditLog {
    path: PathBuf,
    max_size_bytes: u64,
}

impl AuditLog {
    /// Create a log handle for `path`, rotating once it exceeds
    /// `max_size_bytes` (0 disables rotation).
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
            std::fs::create_dir_all(parent).map_err(|e| AuditError::WriteFailed(e.to_string()))?;
        }

        // Check size and rotate if needed. The rotated file's tip hash is
        // captured first and carried into the new file's first record, so the
        // chains stay linked across a rotation (verifiable while the rotated
        // file is still present).
        let mut rotated_tip: Option<String> = None;
        if let Ok(metadata) = std::fs::metadata(&self.path) {
            if metadata.len() > self.max_size_bytes && self.max_size_bytes > 0 {
                rotated_tip = Some(
                    self.read_chained()?
                        .last()
                        .map(|last| last.record_hash.clone())
                        .unwrap_or_default(),
                );
                self.rotate()
                    .map_err(|e| AuditError::WriteFailed(e.to_string()))?;
            }
        }

        // Determine the chain head (seq + prev_hash): after rotation the new
        // file restarts at seq 0 anchored to the rotated tip; otherwise it
        // continues from the existing log.
        let (seq, prev_hash) = match rotated_tip {
            Some(tip) => (0, tip),
            None => match self.read_chained()?.last() {
                Some(last) => (last.seq + 1, last.record_hash.clone()),
                None => (0, String::new()),
            },
        };

        let record_hash = ChainedRecord::compute_hash(record, seq, &prev_hash)?;
        let chained = ChainedRecord {
            record: record.clone(),
            seq,
            prev_hash,
            record_hash,
        };

        // Encode the chained record as CBOR
        let mut cbor_bytes = Vec::new();
        ciborium::into_writer(&chained, &mut cbor_bytes)
            .map_err(|e| AuditError::WriteFailed(e.to_string()))?;

        // Write length-prefixed record. On Unix the log is created 0600 so
        // audit material is only readable by the owning user.
        let mut open_opts = std::fs::OpenOptions::new();
        open_opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_opts.mode(0o600);
        }
        let mut file = open_opts
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
        Ok(self.read_chained()?.into_iter().map(|c| c.record).collect())
    }

    /// Read all records together with their hash-chain metadata.
    fn read_chained(&self) -> Result<Vec<ChainedRecord>, AuditError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let data = std::fs::read(&self.path).map_err(|e| AuditError::Corrupted(e.to_string()))?;

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

            let record: ChainedRecord = ciborium::from_reader(&data[offset..offset + len])
                .map_err(|e| AuditError::Corrupted(e.to_string()))?;
            records.push(record);
            offset += len;
        }

        Ok(records)
    }

    /// Verify the integrity of the hash chain.
    ///
    /// Detects tampering (a record's content or metadata was altered),
    /// reordering (records swapped), and interior deletion (a record removed
    /// from the middle). Returns [`AuditError::IntegrityViolation`] describing
    /// the first break found.
    ///
    /// # Limitations (read before trusting a "verified" log)
    ///
    /// - **No external anchor.** The chain is entirely self-contained: an
    ///   attacker with write access to the log file can rewrite the whole
    ///   history from genesis (recomputing every hash) and the result will
    ///   verify cleanly. This method only proves the file is *internally*
    ///   consistent, not that it is the history that was actually written.
    ///   Detecting a full rewrite requires an out-of-band anchor (e.g.
    ///   periodically recording the tip hash on another host); none is built
    ///   in.
    /// - **Trailing truncation is invisible.** Removing records from the end
    ///   leaves a prefix that still verifies; only mid-file truncation or a
    ///   partial final record is caught here or skipped by the reader.
    /// - **Rotation bounds continuity.** The first record after a rotation
    ///   carries the rotated file's tip hash in `prev_hash` (see
    ///   [`AuditLog::verify_chain`]'s genesis handling and the rotation
    ///   notes on `rotate`), but this method verifies one file at a time and
    ///   does not follow that link; a genesis `prev_hash` is accepted
    ///   without validation since its predecessor file may no longer exist.
    pub fn verify_chain(&self) -> Result<(), AuditError> {
        let records = self.read_chained()?;

        let mut expected_prev = String::new();

        for (i, cr) in records.iter().enumerate() {
            // The chain is dense and zero-based, so the expected sequence
            // number is the record's position.
            let expected_seq = i as u64;
            if cr.record_hash.is_empty() {
                return Err(AuditError::IntegrityViolation(format!(
                    "record {i} is not part of the hash chain (legacy/unchained)"
                )));
            }

            // Internal integrity: recompute the stored hash.
            let computed = ChainedRecord::compute_hash(&cr.record, cr.seq, &cr.prev_hash)?;
            if computed != cr.record_hash {
                return Err(AuditError::IntegrityViolation(format!(
                    "record {i} hash mismatch (tampered content)"
                )));
            }

            // Linkage: prev_hash must point at the previous record's hash.
            // The genesis record (i == 0) is exempt: its prev_hash is either
            // empty (fresh log) or the tip hash of a rotated predecessor
            // file, which cannot be validated from this file alone.
            if i > 0 && cr.prev_hash != expected_prev {
                return Err(AuditError::IntegrityViolation(format!(
                    "record {i} prev_hash does not link to predecessor (reorder/deletion)"
                )));
            }

            // Contiguity: sequence numbers must be dense and increasing.
            if cr.seq != expected_seq {
                return Err(AuditError::IntegrityViolation(format!(
                    "record {i} has seq {} (expected {expected_seq})",
                    cr.seq
                )));
            }

            expected_prev = cr.record_hash.clone();
        }

        Ok(())
    }

    /// Rotate the current log out of the way (rename to `<name>.log.1`).
    ///
    /// # Limitations
    ///
    /// - **Only one generation is kept.** Rotation overwrites any previous
    ///   `.log.1` file, so history older than one rotation is permanently
    ///   lost. Archive rotated files externally if long-term retention is
    ///   required.
    /// - **The rotated file is orphaned from verification.** After rotation
    ///   the active log restarts at seq 0. Its first record's `prev_hash`
    ///   carries the rotated file's tip hash, linking the two chains, but
    ///   [`AuditLog::verify_chain`] operates on the active file only and does
    ///   not follow or verify that link — the rotated file must be checked
    ///   (and the tip compared) out of band.
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
    fn verify_chain_good() {
        let dir = TempDir::new().unwrap();
        let log = AuditLog::new(dir.path().join("audit.log"), 10 * 1024 * 1024);

        for _ in 0..5 {
            log.append(&sample_record()).unwrap();
        }

        assert_eq!(log.read_all().unwrap().len(), 5);
        log.verify_chain().expect("chain should verify");
    }

    #[test]
    fn verify_chain_detects_tampering() {
        let path;
        {
            let dir = TempDir::new().unwrap();
            path = dir.path().join("audit.log");
            let log = AuditLog::new(path.clone(), 10 * 1024 * 1024);
            log.append(&sample_record()).unwrap();
            log.append(&sample_record()).unwrap();
            log.append(&sample_record()).unwrap();

            // Tamper: flip a byte somewhere inside the file body.
            let mut bytes = std::fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xFF;
            std::fs::write(&path, &bytes).unwrap();

            let log2 = AuditLog::new(path.clone(), 10 * 1024 * 1024);
            let result = log2.verify_chain();
            assert!(
                result.is_err(),
                "tampered chain should fail verification: {result:?}"
            );
            assert!(matches!(
                result,
                Err(AuditError::IntegrityViolation(_)) | Err(AuditError::Corrupted(_))
            ));
        }
    }

    #[test]
    fn verify_chain_detects_reorder() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("audit.log");
        let log = AuditLog::new(path.clone(), 10 * 1024 * 1024);
        log.append(&sample_record()).unwrap();
        log.append(&sample_record()).unwrap();

        // Split the file into its two framed records and swap them.
        let data = std::fs::read(&path).unwrap();
        let len0 = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
        let rec0 = &data[0..4 + len0];
        let rec1 = &data[4 + len0..];
        let mut swapped = Vec::new();
        swapped.extend_from_slice(rec1);
        swapped.extend_from_slice(rec0);
        std::fs::write(&path, &swapped).unwrap();

        let result = log.verify_chain();
        assert!(result.is_err(), "reordered chain should fail: {result:?}");
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

    #[test]
    fn rotation_carries_tip_hash_and_chain_verifies() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("audit.log");
        // Tiny max size: every append after the first triggers rotation.
        let log = AuditLog::new(path.clone(), 1);

        log.append(&sample_record()).unwrap();
        log.verify_chain().unwrap();

        // Capture the tip of the current file, then trigger rotation.
        let rotated_tip = {
            let recs = log.read_chained().unwrap();
            recs.last().unwrap().record_hash.clone()
        };
        log.append(&sample_record()).unwrap();

        // The new active file restarts at seq 0 with prev_hash anchored to
        // the rotated file's tip, and still verifies.
        let recs = log.read_chained().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].seq, 0);
        assert_eq!(recs[0].prev_hash, rotated_tip);
        assert!(!recs[0].prev_hash.is_empty());
        log.verify_chain().expect("post-rotation chain should verify");

        // The rotated file remains verifiable on its own.
        let rotated = AuditLog::new(dir.path().join("audit.log.1"), 0);
        rotated.verify_chain().expect("rotated file should verify");
    }
}
