//! Fleet-scale canary health probe for Ganglion.
//!
//! A minimal, fast health check designed for "is this robot responsive?"
//! polling across an entire fleet. Returns structured pass/fail/degrade
//! results within a strict time budget, making it suitable for
//! fleet-scale monitoring dashboards and alerting.
//!
//! When compiled to a WASM component this uses `diagnostics-collect`,
//! `network-probe`, and `metrics-emit` host interfaces. As a native
//! library the probe evaluation logic is testable without host access.
//!
//! The design spec designates this as the Go reference capability
//! (TinyGo). The Rust crate implements the canonical logic; a Go
//! example project would demonstrate the authoring pathway for a
//! fourth language.

use serde::{Deserialize, Serialize};

/// Overall health status of a robot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// All checks passed.
    Healthy,
    /// Some checks showed degradation but the robot is functional.
    Degraded,
    /// One or more critical checks failed.
    Unhealthy,
    /// The probe could not complete (e.g. timeout, unreachable).
    Unreachable,
}

/// A single health check within the probe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCheck {
    /// Check name (e.g. "memory", "disk", "uptime", "ros_nodes").
    pub name: String,
    /// Whether this check passed.
    pub passed: bool,
    /// Human-readable detail about the result.
    pub detail: String,
    /// Numeric value if applicable (e.g. memory usage percentage).
    pub value: Option<f64>,
    /// Threshold that was evaluated against, if applicable.
    pub threshold: Option<f64>,
}

/// Canary probe result for a single robot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryResult {
    /// ISO 8601 timestamp of probe execution.
    pub timestamp: String,
    /// Overall health status.
    pub status: HealthStatus,
    /// Individual check results.
    pub checks: Vec<HealthCheck>,
    /// Total probe execution time in milliseconds.
    pub elapsed_ms: u64,
    /// Number of checks that passed.
    pub passed: usize,
    /// Number of checks that failed.
    pub failed: usize,
    /// Total number of checks run.
    pub total: usize,
}

/// Thresholds for the canary probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeThresholds {
    /// Memory usage above this percentage triggers degraded status.
    pub memory_warn_pct: f64,
    /// Memory usage above this percentage triggers unhealthy status.
    pub memory_crit_pct: f64,
    /// Disk usage above this percentage triggers degraded status.
    pub disk_warn_pct: f64,
    /// Disk usage above this percentage triggers unhealthy status.
    pub disk_crit_pct: f64,
    /// Minimum uptime in seconds to consider healthy.
    pub min_uptime_secs: u64,
}

impl Default for ProbeThresholds {
    fn default() -> Self {
        Self {
            memory_warn_pct: 80.0,
            memory_crit_pct: 95.0,
            disk_warn_pct: 85.0,
            disk_crit_pct: 95.0,
            min_uptime_secs: 60,
        }
    }
}

/// Check memory usage against thresholds.
pub fn check_memory(
    total_mb: Option<u64>,
    available_mb: Option<u64>,
    thresholds: &ProbeThresholds,
) -> HealthCheck {
    match (total_mb, available_mb) {
        (Some(total), Some(available)) if total > 0 => {
            let used_pct = ((total - available) as f64 / total as f64) * 100.0;
            let passed = used_pct < thresholds.memory_crit_pct;
            let detail = if used_pct >= thresholds.memory_crit_pct {
                format!("CRITICAL: {used_pct:.1}% memory used")
            } else if used_pct >= thresholds.memory_warn_pct {
                format!("WARNING: {used_pct:.1}% memory used")
            } else {
                format!("{used_pct:.1}% memory used")
            };
            HealthCheck {
                name: "memory".into(),
                passed,
                detail,
                value: Some(used_pct),
                threshold: Some(thresholds.memory_crit_pct),
            }
        }
        _ => HealthCheck {
            name: "memory".into(),
            passed: true,
            detail: "memory data unavailable — skipped".into(),
            value: None,
            threshold: None,
        },
    }
}

/// Check disk usage against thresholds.
pub fn check_disk(
    total_mb: Option<u64>,
    used_mb: Option<u64>,
    thresholds: &ProbeThresholds,
) -> HealthCheck {
    match (total_mb, used_mb) {
        (Some(total), Some(used)) if total > 0 => {
            let used_pct = (used as f64 / total as f64) * 100.0;
            let passed = used_pct < thresholds.disk_crit_pct;
            let detail = if used_pct >= thresholds.disk_crit_pct {
                format!("CRITICAL: {used_pct:.1}% disk used")
            } else if used_pct >= thresholds.disk_warn_pct {
                format!("WARNING: {used_pct:.1}% disk used")
            } else {
                format!("{used_pct:.1}% disk used")
            };
            HealthCheck {
                name: "disk".into(),
                passed,
                detail,
                value: Some(used_pct),
                threshold: Some(thresholds.disk_crit_pct),
            }
        }
        _ => HealthCheck {
            name: "disk".into(),
            passed: true,
            detail: "disk data unavailable — skipped".into(),
            value: None,
            threshold: None,
        },
    }
}

/// Check system uptime against minimum threshold.
pub fn check_uptime(uptime_secs: Option<u64>, thresholds: &ProbeThresholds) -> HealthCheck {
    match uptime_secs {
        Some(secs) => {
            let passed = secs >= thresholds.min_uptime_secs;
            let detail = if passed {
                format!("uptime {secs}s (ok)")
            } else {
                format!(
                    "uptime {secs}s < {}s minimum — possible recent reboot",
                    thresholds.min_uptime_secs
                )
            };
            HealthCheck {
                name: "uptime".into(),
                passed,
                detail,
                value: Some(secs as f64),
                threshold: Some(thresholds.min_uptime_secs as f64),
            }
        }
        None => HealthCheck {
            name: "uptime".into(),
            passed: true,
            detail: "uptime data unavailable — skipped".into(),
            value: None,
            threshold: None,
        },
    }
}

/// Check that the robot is reachable (simple liveness check).
pub fn check_reachable(is_reachable: bool) -> HealthCheck {
    HealthCheck {
        name: "reachable".into(),
        passed: is_reachable,
        detail: if is_reachable {
            "robot responded to probe".into()
        } else {
            "robot did not respond".into()
        },
        value: None,
        threshold: None,
    }
}

/// Evaluate a set of health checks and produce a canary result.
pub fn evaluate(checks: Vec<HealthCheck>, timestamp: &str, elapsed_ms: u64) -> CanaryResult {
    let total = checks.len();
    let passed = checks.iter().filter(|c| c.passed).count();
    let failed = total - passed;

    // Determine overall status
    let has_unreachable = checks.iter().any(|c| c.name == "reachable" && !c.passed);

    let status = if has_unreachable {
        HealthStatus::Unreachable
    } else if failed > 0 {
        HealthStatus::Unhealthy
    } else {
        // Check for warnings: passed checks where detail contains WARNING
        let has_warning = checks
            .iter()
            .any(|c| c.passed && c.detail.contains("WARNING"));
        if has_warning {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        }
    };

    CanaryResult {
        timestamp: timestamp.into(),
        status,
        checks,
        elapsed_ms,
        passed,
        failed,
        total,
    }
}

/// Format a canary result as a compact one-line summary for fleet dashboards.
pub fn format_oneliner(result: &CanaryResult) -> String {
    let status_str = match result.status {
        HealthStatus::Healthy => "OK",
        HealthStatus::Degraded => "DEGRADED",
        HealthStatus::Unhealthy => "UNHEALTHY",
        HealthStatus::Unreachable => "UNREACHABLE",
    };
    format!(
        "[{}] {}/{} checks passed ({}ms)",
        status_str, result.passed, result.total, result.elapsed_ms
    )
}

/// Format a canary result as a detailed report.
pub fn format_report(result: &CanaryResult) -> String {
    let mut out = String::new();
    out.push_str("Canary Probe Report\n");
    out.push_str(&"=".repeat(50));
    out.push('\n');

    out.push_str(&format!("\nTimestamp: {}\n", result.timestamp));
    out.push_str(&format!("Status:   {:?}\n", result.status));
    out.push_str(&format!(
        "Checks:   {}/{} passed\n",
        result.passed, result.total
    ));
    out.push_str(&format!("Elapsed:  {}ms\n", result.elapsed_ms));

    out.push_str("\n## Checks\n");
    for check in &result.checks {
        let icon = if check.passed { "+" } else { "!" };
        out.push_str(&format!("  [{icon}] {}: {}\n", check.name, check.detail));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_robot() {
        let thresholds = ProbeThresholds::default();
        let checks = vec![
            check_reachable(true),
            check_memory(Some(8192), Some(6000), &thresholds),
            check_disk(Some(102400), Some(40000), &thresholds),
            check_uptime(Some(86400), &thresholds),
        ];
        let result = evaluate(checks, "2026-04-23T12:00:00Z", 15);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.passed, 4);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn degraded_memory() {
        let thresholds = ProbeThresholds::default();
        let checks = vec![
            check_reachable(true),
            check_memory(Some(8192), Some(1200), &thresholds), // ~85% used
            check_disk(Some(102400), Some(40000), &thresholds),
            check_uptime(Some(86400), &thresholds),
        ];
        let result = evaluate(checks, "2026-04-23T12:00:00Z", 15);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn unhealthy_disk() {
        let thresholds = ProbeThresholds::default();
        let checks = vec![
            check_reachable(true),
            check_memory(Some(8192), Some(6000), &thresholds),
            check_disk(Some(102400), Some(99000), &thresholds), // ~96.7% used
            check_uptime(Some(86400), &thresholds),
        ];
        let result = evaluate(checks, "2026-04-23T12:00:00Z", 15);
        assert_eq!(result.status, HealthStatus::Unhealthy);
        assert_eq!(result.failed, 1);
    }

    #[test]
    fn unreachable_robot() {
        let checks = vec![check_reachable(false)];
        let result = evaluate(checks, "2026-04-23T12:00:00Z", 5000);
        assert_eq!(result.status, HealthStatus::Unreachable);
    }

    #[test]
    fn recent_reboot_detected() {
        let thresholds = ProbeThresholds::default();
        let check = check_uptime(Some(30), &thresholds); // 30s < 60s minimum
        assert!(!check.passed);
        assert!(check.detail.contains("recent reboot"));
    }

    #[test]
    fn missing_data_skipped() {
        let thresholds = ProbeThresholds::default();
        let mem = check_memory(None, None, &thresholds);
        let disk = check_disk(None, None, &thresholds);
        let uptime = check_uptime(None, &thresholds);
        assert!(mem.passed);
        assert!(disk.passed);
        assert!(uptime.passed);
        assert!(mem.detail.contains("skipped"));
    }

    #[test]
    fn format_oneliner_healthy() {
        let result = CanaryResult {
            timestamp: "2026-04-23T12:00:00Z".into(),
            status: HealthStatus::Healthy,
            checks: Vec::new(),
            elapsed_ms: 12,
            passed: 4,
            failed: 0,
            total: 4,
        };
        let line = format_oneliner(&result);
        assert!(line.contains("[OK]"));
        assert!(line.contains("4/4"));
        assert!(line.contains("12ms"));
    }

    #[test]
    fn format_report_contains_sections() {
        let thresholds = ProbeThresholds::default();
        let checks = vec![
            check_reachable(true),
            check_memory(Some(8192), Some(6000), &thresholds),
        ];
        let result = evaluate(checks, "2026-04-23T12:00:00Z", 15);
        let text = format_report(&result);
        assert!(text.contains("Canary Probe Report"));
        assert!(text.contains("Healthy"));
        assert!(text.contains("## Checks"));
        assert!(text.contains("memory"));
        assert!(text.contains("reachable"));
    }

    #[test]
    fn serialization_roundtrip() {
        let result = CanaryResult {
            timestamp: "2026-04-23T12:00:00Z".into(),
            status: HealthStatus::Degraded,
            checks: vec![HealthCheck {
                name: "test".into(),
                passed: true,
                detail: "ok".into(),
                value: Some(42.0),
                threshold: Some(90.0),
            }],
            elapsed_ms: 25,
            passed: 1,
            failed: 0,
            total: 1,
        };
        let json = serde_json::to_string(&result).unwrap();
        let loaded: CanaryResult = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.status, HealthStatus::Degraded);
        assert_eq!(loaded.checks.len(), 1);
        assert_eq!(loaded.checks[0].name, "test");
    }

    #[test]
    fn thresholds_customizable() {
        let thresholds = ProbeThresholds {
            memory_warn_pct: 50.0,
            memory_crit_pct: 70.0,
            disk_warn_pct: 60.0,
            disk_crit_pct: 80.0,
            min_uptime_secs: 300,
        };
        // 75% memory used — above custom crit threshold of 70%
        let check = check_memory(Some(8000), Some(2000), &thresholds);
        assert!(!check.passed);
        assert!(check.detail.contains("CRITICAL"));
    }

    #[test]
    fn default_thresholds() {
        let t = ProbeThresholds::default();
        assert_eq!(t.memory_warn_pct, 80.0);
        assert_eq!(t.memory_crit_pct, 95.0);
        assert_eq!(t.disk_warn_pct, 85.0);
        assert_eq!(t.disk_crit_pct, 95.0);
        assert_eq!(t.min_uptime_secs, 60);
    }
}

/// WASM component entry point — bridges the `ganglion-capability` world's
/// `run` export to this crate's canonical logic (wasm32 builds only; see
/// `component.rs`). Native builds and tests are unaffected.
#[cfg(target_arch = "wasm32")]
mod component;
