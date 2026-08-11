//! Diagnostic bundle v2 — comprehensive system and ROS diagnostics.
//!
//! Collects system information, journald excerpts, dmesg, systemd unit status,
//! ROS node graph, and network state into a single structured bundle.
//!
//! When compiled to a WASM component, this uses WIT diagnostics-collect,
//! process/spawn, and fs/bounded imports.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A complete diagnostic bundle collected from a robot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    /// When the bundle was collected (ISO 8601).
    pub timestamp: String,
    /// Hostname of the robot.
    pub hostname: String,
    /// System information section.
    pub system: SystemSection,
    /// Journal/log excerpts.
    pub journal: JournalSection,
    /// Systemd unit status.
    pub units: Vec<UnitStatus>,
    /// Network state.
    pub network: NetworkSection,
    /// ROS 2 graph state.
    pub ros_graph: RosGraphSection,
    /// Custom diagnostic data from plugins.
    pub custom: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSection {
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub uptime_secs: u64,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub cpu_count: u32,
    pub load_average: [f64; 3],
    pub disk_usage: Vec<DiskUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    pub mount_point: String,
    pub total_gb: f64,
    pub used_gb: f64,
    pub available_gb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalSection {
    /// Last N lines from journald (or syslog fallback).
    pub system_log: Vec<String>,
    /// Last N lines from dmesg.
    pub dmesg: Vec<String>,
    /// ROS-specific log lines.
    pub ros_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitStatus {
    pub name: String,
    pub active_state: String,
    pub sub_state: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSection {
    pub interfaces: Vec<NetworkInterface>,
    pub routes: Vec<String>,
    pub dns_servers: Vec<String>,
    pub active_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub mac: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosGraphSection {
    pub nodes: Vec<RosNode>,
    pub topics: Vec<RosTopic>,
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosNode {
    pub name: String,
    pub namespace: String,
    pub publishers: Vec<String>,
    pub subscribers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosTopic {
    pub name: String,
    pub message_type: String,
    pub publisher_count: u32,
    pub subscriber_count: u32,
}

/// Severity level for diagnostic checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Ok,
    Warning,
    Error,
    Critical,
}

/// A diagnostic check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticCheck {
    pub name: String,
    pub severity: Severity,
    pub message: String,
}

/// Run automated diagnostic checks on a bundle.
pub fn analyze_bundle(bundle: &DiagnosticBundle) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // Memory pressure
    if bundle.system.memory_total_mb > 0 {
        let usage_pct =
            (bundle.system.memory_used_mb as f64 / bundle.system.memory_total_mb as f64) * 100.0;
        let severity = if usage_pct > 95.0 {
            Severity::Critical
        } else if usage_pct > 85.0 {
            Severity::Warning
        } else {
            Severity::Ok
        };
        checks.push(DiagnosticCheck {
            name: "memory_usage".into(),
            severity,
            message: format!(
                "{usage_pct:.1}% memory used ({}/{} MB)",
                bundle.system.memory_used_mb, bundle.system.memory_total_mb
            ),
        });
    }

    // Disk space
    for disk in &bundle.system.disk_usage {
        if disk.total_gb > 0.0 {
            let usage_pct = (disk.used_gb / disk.total_gb) * 100.0;
            let severity = if usage_pct > 95.0 {
                Severity::Critical
            } else if usage_pct > 85.0 {
                Severity::Warning
            } else {
                Severity::Ok
            };
            checks.push(DiagnosticCheck {
                name: format!("disk_{}", disk.mount_point),
                severity,
                message: format!(
                    "{usage_pct:.1}% used on {} ({:.1}/{:.1} GB)",
                    disk.mount_point, disk.used_gb, disk.total_gb
                ),
            });
        }
    }

    // Load average
    if bundle.system.cpu_count > 0 {
        let load_per_cpu = bundle.system.load_average[0] / bundle.system.cpu_count as f64;
        let severity = if load_per_cpu > 2.0 {
            Severity::Critical
        } else if load_per_cpu > 1.0 {
            Severity::Warning
        } else {
            Severity::Ok
        };
        checks.push(DiagnosticCheck {
            name: "cpu_load".into(),
            severity,
            message: format!(
                "load avg {:.2} ({:.2} per CPU, {} CPUs)",
                bundle.system.load_average[0], load_per_cpu, bundle.system.cpu_count
            ),
        });
    }

    // Failed systemd units
    let failed_units: Vec<_> = bundle
        .units
        .iter()
        .filter(|u| u.active_state == "failed")
        .collect();
    if !failed_units.is_empty() {
        checks.push(DiagnosticCheck {
            name: "systemd_failures".into(),
            severity: Severity::Error,
            message: format!(
                "{} failed unit(s): {}",
                failed_units.len(),
                failed_units
                    .iter()
                    .map(|u| u.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    } else {
        checks.push(DiagnosticCheck {
            name: "systemd_failures".into(),
            severity: Severity::Ok,
            message: "no failed units".into(),
        });
    }

    // Network interfaces
    let down_ifaces: Vec<_> = bundle
        .network
        .interfaces
        .iter()
        .filter(|i| i.state == "down" && i.name != "lo")
        .collect();
    if !down_ifaces.is_empty() {
        checks.push(DiagnosticCheck {
            name: "network_interfaces".into(),
            severity: Severity::Warning,
            message: format!(
                "{} interface(s) down: {}",
                down_ifaces.len(),
                down_ifaces
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }

    checks
}

/// Format a diagnostic bundle as a human-readable report.
pub fn format_report(bundle: &DiagnosticBundle, checks: &[DiagnosticCheck]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Diagnostic Bundle — {} @ {}\n",
        bundle.hostname, bundle.timestamp
    ));
    out.push_str(&"═".repeat(60));
    out.push('\n');

    // System
    out.push_str("\n## System\n");
    out.push_str(&format!(
        "  OS: {} ({})\n  Kernel: {}\n  Uptime: {}s\n  CPUs: {}\n  Memory: {}/{} MB\n",
        bundle.system.os,
        bundle.system.arch,
        bundle.system.kernel,
        bundle.system.uptime_secs,
        bundle.system.cpu_count,
        bundle.system.memory_used_mb,
        bundle.system.memory_total_mb,
    ));

    // Checks
    out.push_str("\n## Diagnostic Checks\n");
    for check in checks {
        let icon = match check.severity {
            Severity::Ok => "[OK]",
            Severity::Warning => "[WARN]",
            Severity::Error => "[ERR]",
            Severity::Critical => "[CRIT]",
        };
        out.push_str(&format!("  {icon} {}: {}\n", check.name, check.message));
    }

    // ROS graph summary
    if !bundle.ros_graph.nodes.is_empty() {
        out.push_str(&format!(
            "\n## ROS 2 Graph\n  Nodes: {}\n  Topics: {}\n  Services: {}\n",
            bundle.ros_graph.nodes.len(),
            bundle.ros_graph.topics.len(),
            bundle.ros_graph.services.len(),
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle() -> DiagnosticBundle {
        DiagnosticBundle {
            timestamp: "2026-04-23T12:00:00Z".into(),
            hostname: "robot-01".into(),
            system: SystemSection {
                os: "Ubuntu 24.04".into(),
                kernel: "6.8.0-35-generic".into(),
                arch: "aarch64".into(),
                uptime_secs: 86400,
                memory_total_mb: 8192,
                memory_used_mb: 6000,
                cpu_count: 4,
                load_average: [2.5, 2.0, 1.5],
                disk_usage: vec![DiskUsage {
                    mount_point: "/".into(),
                    total_gb: 100.0,
                    used_gb: 75.0,
                    available_gb: 25.0,
                }],
            },
            journal: JournalSection {
                system_log: vec!["Apr 23 12:00:00 robot-01 kernel: boot complete".into()],
                dmesg: vec!["[    0.000000] Linux version 6.8.0".into()],
                ros_log: vec!["[INFO] /robot_state: initialized".into()],
            },
            units: vec![
                UnitStatus {
                    name: "ros2.service".into(),
                    active_state: "active".into(),
                    sub_state: "running".into(),
                    description: "ROS 2 daemon".into(),
                },
                UnitStatus {
                    name: "ganglion-agent.service".into(),
                    active_state: "active".into(),
                    sub_state: "running".into(),
                    description: "Ganglion agent".into(),
                },
            ],
            network: NetworkSection {
                interfaces: vec![
                    NetworkInterface {
                        name: "eth0".into(),
                        mac: "aa:bb:cc:dd:ee:ff".into(),
                        ipv4: vec!["192.168.1.100".into()],
                        ipv6: vec![],
                        state: "up".into(),
                    },
                    NetworkInterface {
                        name: "wlan0".into(),
                        mac: "11:22:33:44:55:66".into(),
                        ipv4: vec![],
                        ipv6: vec![],
                        state: "down".into(),
                    },
                ],
                routes: vec!["default via 192.168.1.1".into()],
                dns_servers: vec!["8.8.8.8".into()],
                active_connections: 12,
            },
            ros_graph: RosGraphSection {
                nodes: vec![RosNode {
                    name: "/robot_state".into(),
                    namespace: "/".into(),
                    publishers: vec!["/diagnostics".into()],
                    subscribers: vec!["/cmd_vel".into()],
                }],
                topics: vec![RosTopic {
                    name: "/diagnostics".into(),
                    message_type: "diagnostic_msgs/msg/DiagnosticArray".into(),
                    publisher_count: 1,
                    subscriber_count: 0,
                }],
                services: vec!["/robot_state/get_state".into()],
            },
            custom: BTreeMap::new(),
        }
    }

    #[test]
    fn analyze_normal_system() {
        let bundle = sample_bundle();
        let checks = analyze_bundle(&bundle);
        assert!(!checks.is_empty());

        // Memory at ~73% — should be OK
        let mem = checks.iter().find(|c| c.name == "memory_usage").unwrap();
        assert_eq!(mem.severity, Severity::Ok);
    }

    #[test]
    fn analyze_high_memory() {
        let mut bundle = sample_bundle();
        bundle.system.memory_used_mb = 7900; // ~96%
        let checks = analyze_bundle(&bundle);
        let mem = checks.iter().find(|c| c.name == "memory_usage").unwrap();
        assert_eq!(mem.severity, Severity::Critical);
    }

    #[test]
    fn analyze_disk_warning() {
        let mut bundle = sample_bundle();
        bundle.system.disk_usage[0].used_gb = 90.0; // 90%
        let checks = analyze_bundle(&bundle);
        let disk = checks.iter().find(|c| c.name == "disk_/").unwrap();
        assert_eq!(disk.severity, Severity::Warning);
    }

    #[test]
    fn analyze_failed_units() {
        let mut bundle = sample_bundle();
        bundle.units.push(UnitStatus {
            name: "broken.service".into(),
            active_state: "failed".into(),
            sub_state: "failed".into(),
            description: "A broken service".into(),
        });
        let checks = analyze_bundle(&bundle);
        let units = checks
            .iter()
            .find(|c| c.name == "systemd_failures")
            .unwrap();
        assert_eq!(units.severity, Severity::Error);
        assert!(units.message.contains("broken.service"));
    }

    #[test]
    fn analyze_down_interface() {
        let bundle = sample_bundle();
        let checks = analyze_bundle(&bundle);
        let net = checks
            .iter()
            .find(|c| c.name == "network_interfaces")
            .unwrap();
        assert_eq!(net.severity, Severity::Warning);
        assert!(net.message.contains("wlan0"));
    }

    #[test]
    fn format_report_readable() {
        let bundle = sample_bundle();
        let checks = analyze_bundle(&bundle);
        let report = format_report(&bundle, &checks);
        assert!(report.contains("robot-01"));
        assert!(report.contains("Ubuntu 24.04"));
        assert!(report.contains("[OK]"));
        assert!(report.contains("ROS 2 Graph"));
    }

    #[test]
    fn bundle_json_roundtrip() {
        let bundle = sample_bundle();
        let json = serde_json::to_string(&bundle).unwrap();
        let loaded: DiagnosticBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.hostname, "robot-01");
        assert_eq!(loaded.system.cpu_count, 4);
        assert_eq!(loaded.ros_graph.nodes.len(), 1);
    }
}

/// WASM component entry point — bridges the `ganglion-capability` world's
/// `run` export to this crate's canonical logic (wasm32 builds only; see
/// `component.rs`). Native builds and tests are unaffected.
#[cfg(target_arch = "wasm32")]
mod component;
