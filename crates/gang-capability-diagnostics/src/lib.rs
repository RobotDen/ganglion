//! Reference diagnostic capability for Ganglion.
//!
//! This is the simplest WASM capability in the workspace, demonstrating
//! the collect-aggregate-return pattern that all capabilities follow.
//! When running as a WASM component it calls `diagnostics-collect` host
//! functions to gather real system data; as a native library the
//! [`collect`] function returns placeholder values suitable for testing
//! and documentation.

use serde::{Deserialize, Serialize};

/// A single disk mount entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskEntry {
    /// Mount point path (e.g. `/`, `/data`).
    pub mount: String,
    /// Total capacity in megabytes.
    pub total_mb: u64,
    /// Used space in megabytes.
    pub used_mb: u64,
}

/// Lightweight system diagnostic report.
///
/// Fields are intentionally simple so the struct can be constructed
/// from the `diagnostics-collect` WIT host calls or from test defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticReport {
    /// ISO 8601 timestamp of collection.
    pub timestamp: String,
    /// Hostname of the machine.
    pub hostname: String,
    /// Operating system description.
    pub os: String,
    /// CPU architecture (e.g. `aarch64`, `x86_64`).
    pub arch: String,
    /// System uptime in seconds, if available.
    pub uptime_secs: Option<u64>,
    /// Total physical memory in megabytes, if available.
    pub memory_total_mb: Option<u64>,
    /// Available (free + reclaimable) memory in megabytes, if available.
    pub memory_available_mb: Option<u64>,
    /// Mounted disk partitions with usage.
    pub disk_usage: Vec<DiskEntry>,
}

/// Collect a diagnostic report with placeholder/default values.
///
/// In a real WASM component this function would call the
/// `diagnostics-collect::system-info` and `diagnostics-collect::network-state`
/// host imports, parse the returned `diagnostic-entry` lists, and aggregate
/// them into a [`DiagnosticReport`].  The native-library version returns
/// static defaults to demonstrate the pattern without requiring host access.
pub fn collect() -> DiagnosticReport {
    DiagnosticReport {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        hostname: "unknown".to_string(),
        os: "unknown".to_string(),
        arch: "unknown".to_string(),
        uptime_secs: None,
        memory_total_mb: None,
        memory_available_mb: None,
        disk_usage: Vec::new(),
    }
}

/// Format a [`DiagnosticReport`] as a human-readable summary.
pub fn format_report(report: &DiagnosticReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Diagnostic Report - {} @ {}\n",
        report.hostname, report.timestamp
    ));
    out.push_str(&"=".repeat(50));
    out.push('\n');

    // System section
    out.push_str("\n## System\n");
    out.push_str(&format!("  OS:   {}\n", report.os));
    out.push_str(&format!("  Arch: {}\n", report.arch));

    match report.uptime_secs {
        Some(s) => out.push_str(&format!("  Uptime: {s}s\n")),
        None => out.push_str("  Uptime: n/a\n"),
    }

    // Memory section
    out.push_str("\n## Memory\n");
    match (report.memory_total_mb, report.memory_available_mb) {
        (Some(total), Some(avail)) => {
            out.push_str(&format!("  Total:     {total} MB\n"));
            out.push_str(&format!("  Available: {avail} MB\n"));
        }
        (Some(total), None) => {
            out.push_str(&format!("  Total:     {total} MB\n"));
            out.push_str("  Available: n/a\n");
        }
        _ => {
            out.push_str("  (no memory data)\n");
        }
    }

    // Disk section
    out.push_str("\n## Disk\n");
    if report.disk_usage.is_empty() {
        out.push_str("  (no disk data)\n");
    } else {
        for d in &report.disk_usage {
            let pct = if d.total_mb > 0 {
                (d.used_mb as f64 / d.total_mb as f64) * 100.0
            } else {
                0.0
            };
            out.push_str(&format!(
                "  {}: {}/{} MB ({pct:.1}% used)\n",
                d.mount, d.used_mb, d.total_mb
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_returns_defaults() {
        let report = collect();
        assert_eq!(report.hostname, "unknown");
        assert_eq!(report.arch, "unknown");
        assert!(report.uptime_secs.is_none());
        assert!(report.memory_total_mb.is_none());
        assert!(report.disk_usage.is_empty());
    }

    #[test]
    fn report_construction_with_values() {
        let report = DiagnosticReport {
            timestamp: "2026-04-23T12:00:00Z".into(),
            hostname: "robot-01".into(),
            os: "Ubuntu 24.04".into(),
            arch: "aarch64".into(),
            uptime_secs: Some(86400),
            memory_total_mb: Some(8192),
            memory_available_mb: Some(4096),
            disk_usage: vec![DiskEntry {
                mount: "/".into(),
                total_mb: 102400,
                used_mb: 61440,
            }],
        };
        assert_eq!(report.hostname, "robot-01");
        assert_eq!(report.uptime_secs, Some(86400));
        assert_eq!(report.disk_usage.len(), 1);
        assert_eq!(report.disk_usage[0].used_mb, 61440);
    }

    #[test]
    fn serialization_roundtrip() {
        let report = DiagnosticReport {
            timestamp: "2026-04-23T12:00:00Z".into(),
            hostname: "robot-02".into(),
            os: "Debian 12".into(),
            arch: "x86_64".into(),
            uptime_secs: Some(3600),
            memory_total_mb: Some(16384),
            memory_available_mb: Some(12000),
            disk_usage: vec![
                DiskEntry {
                    mount: "/".into(),
                    total_mb: 512000,
                    used_mb: 256000,
                },
                DiskEntry {
                    mount: "/data".into(),
                    total_mb: 1024000,
                    used_mb: 100000,
                },
            ],
        };

        let json = serde_json::to_string(&report).unwrap();
        let loaded: DiagnosticReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, loaded);
    }

    #[test]
    fn format_report_contains_sections() {
        let report = DiagnosticReport {
            timestamp: "2026-04-23T12:00:00Z".into(),
            hostname: "robot-03".into(),
            os: "Ubuntu 24.04".into(),
            arch: "aarch64".into(),
            uptime_secs: Some(7200),
            memory_total_mb: Some(8192),
            memory_available_mb: Some(2048),
            disk_usage: vec![DiskEntry {
                mount: "/".into(),
                total_mb: 102400,
                used_mb: 81920,
            }],
        };
        let text = format_report(&report);
        assert!(text.contains("robot-03"), "missing hostname");
        assert!(text.contains("## System"), "missing system section");
        assert!(text.contains("## Memory"), "missing memory section");
        assert!(text.contains("## Disk"), "missing disk section");
        assert!(text.contains("8192 MB"), "missing total memory");
        assert!(text.contains("80.0% used"), "missing disk percentage");
    }

    #[test]
    fn format_report_empty_disk() {
        let report = DiagnosticReport {
            timestamp: "2026-04-23T12:00:00Z".into(),
            hostname: "robot-04".into(),
            os: "unknown".into(),
            arch: "unknown".into(),
            uptime_secs: None,
            memory_total_mb: None,
            memory_available_mb: None,
            disk_usage: Vec::new(),
        };
        let text = format_report(&report);
        assert!(text.contains("(no disk data)"), "missing empty-disk text");
        assert!(
            text.contains("(no memory data)"),
            "missing empty-memory text"
        );
        assert!(text.contains("Uptime: n/a"), "missing n/a uptime");
    }

    #[test]
    fn optional_fields_none_serialize() {
        let report = DiagnosticReport {
            timestamp: "2026-04-23T00:00:00Z".into(),
            hostname: "bare".into(),
            os: "none".into(),
            arch: "unknown".into(),
            uptime_secs: None,
            memory_total_mb: None,
            memory_available_mb: None,
            disk_usage: Vec::new(),
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.contains("null"),
            "None fields should serialize as null"
        );

        let loaded: DiagnosticReport = serde_json::from_str(&json).unwrap();
        assert!(loaded.uptime_secs.is_none());
        assert!(loaded.memory_total_mb.is_none());
        assert!(loaded.memory_available_mb.is_none());
    }
}
