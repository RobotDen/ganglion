//! WASM component entry point (wasm32 builds only).
//!
//! Assembles the comprehensive diagnostic bundle from inside the sandbox:
//! system stats via `diagnostics-collect`, journal/dmesg/ROS log excerpts via
//! `logs-stream` (matching whatever source names the host advertises), and a
//! lenient parse of the broker's network state. The bundle then runs through
//! the crate's canonical `analyze_bundle` checks. With `--publish` the full
//! bundle JSON is stored as a content-addressed artifact and the CID is
//! appended to the report. Sections the sandbox cannot observe (systemd
//! units, ROS graph detail) are left empty — the checks tolerate absent data.
//! Args: `[--json] [--publish]`.

use std::collections::BTreeMap;

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::{artifacts_publish, diagnostics_collect, logs_stream};

use crate::{
    DiagnosticBundle, DiskUsage, JournalSection, NetworkInterface, NetworkSection, RosGraphSection,
    SystemSection, analyze_bundle, format_report,
};

struct Component;

fn get(entries: &[diagnostics_collect::DiagnosticEntry], key: &str) -> String {
    entries
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.value.clone())
        .unwrap_or_default()
}

fn get_u64(entries: &[diagnostics_collect::DiagnosticEntry], key: &str) -> u64 {
    get(entries, key).parse().unwrap_or(0)
}

const MB: u64 = 1024 * 1024;
const GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Fetch log lines from the first advertised source whose name contains any
/// of the given fragments. Missing sources yield an empty section, not an
/// error — a bundle from a robot without journald is still a bundle.
fn logs_matching(sources: &[logs_stream::LogSource], fragments: &[&str]) -> Vec<String> {
    sources
        .iter()
        .find(|s| {
            let name = s.name.to_ascii_lowercase();
            fragments.iter().any(|f| name.contains(f))
        })
        .and_then(|s| logs_stream::stream_logs(&s.name, "").ok())
        .unwrap_or_default()
}

/// Lenient parse of the stringified network-state entries into the bundle's
/// network section. The broker returns nested JSON stringified per key; any
/// shape drift degrades to an emptier section rather than an error.
fn network_section(entries: &[diagnostics_collect::DiagnosticEntry]) -> NetworkSection {
    let mut interfaces = Vec::new();
    let mut active_connections = 0u32;

    if let Ok(serde_json::Value::Array(items)) =
        serde_json::from_str::<serde_json::Value>(&get(entries, "interfaces"))
    {
        for item in items {
            let s = |k: &str| {
                item.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let list = |k: &str| -> Vec<String> {
                item.get(k)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            interfaces.push(NetworkInterface {
                name: s("name"),
                mac: s("mac"),
                ipv4: list("ipv4"),
                ipv6: list("ipv6"),
                state: s("state"),
            });
        }
    }
    if let Ok(serde_json::Value::Array(conns)) =
        serde_json::from_str::<serde_json::Value>(&get(entries, "connections"))
    {
        active_connections = conns.len() as u32;
    }

    NetworkSection {
        interfaces,
        routes: Vec::new(),
        dns_servers: Vec::new(),
        active_connections,
    }
}

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let sys = diagnostics_collect::system_info()?;
        let net = diagnostics_collect::network_state().unwrap_or_default();
        let sources = logs_stream::list_sources().unwrap_or_default();

        let disk_total = get_u64(&sys, "disk_total_bytes");
        let disk_avail = get_u64(&sys, "disk_available_bytes");
        let mem_total_mb = get_u64(&sys, "memory_total_bytes") / MB;
        let mem_avail_mb = get_u64(&sys, "memory_available_bytes") / MB;

        let bundle = DiagnosticBundle {
            timestamp: "n/a".to_string(),
            hostname: get(&sys, "hostname"),
            system: SystemSection {
                os: get(&sys, "os"),
                kernel: get(&sys, "os_version"),
                arch: get(&sys, "arch"),
                uptime_secs: get_u64(&sys, "uptime_secs"),
                memory_total_mb: mem_total_mb,
                memory_used_mb: mem_total_mb.saturating_sub(mem_avail_mb),
                cpu_count: get_u64(&sys, "cpu_count") as u32,
                load_average: [0.0, 0.0, 0.0],
                disk_usage: if disk_total > 0 {
                    vec![DiskUsage {
                        mount_point: "/".to_string(),
                        total_gb: disk_total as f64 / GB,
                        used_gb: disk_total.saturating_sub(disk_avail) as f64 / GB,
                        available_gb: disk_avail as f64 / GB,
                    }]
                } else {
                    Vec::new()
                },
            },
            journal: JournalSection {
                system_log: logs_matching(&sources, &["journal", "syslog", "system"]),
                dmesg: logs_matching(&sources, &["dmesg", "kernel"]),
                ros_log: logs_matching(&sources, &["ros"]),
            },
            units: Vec::new(),
            network: network_section(&net),
            ros_graph: RosGraphSection {
                nodes: Vec::new(),
                topics: Vec::new(),
                services: Vec::new(),
            },
            custom: BTreeMap::new(),
        };

        let checks = analyze_bundle(&bundle);

        let mut out = if args.iter().any(|a| a == "--json") {
            serde_json::to_vec(&serde_json::json!({
                "bundle": bundle,
                "checks": checks,
            }))
            .map_err(|e| e.to_string())?
        } else {
            format_report(&bundle, &checks).into_bytes()
        };

        if args.iter().any(|a| a == "--publish") {
            let payload = serde_json::to_vec(&bundle).map_err(|e| e.to_string())?;
            let cid = artifacts_publish::publish(
                &payload,
                Some("diagnostic-bundle.json"),
                Some("application/json"),
            )?;
            out.extend_from_slice(format!("\nartifact: {cid}\n").as_bytes());
        }

        Ok(out)
    }
}

export!(Component);
