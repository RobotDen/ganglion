//! WASM component entry point (wasm32 builds only).
//!
//! The canary's job is one cheap answer: *is this robot healthy right now?*
//! The guest gathers system stats via `diagnostics-collect`, treats a
//! successful `ros-interface::list-ros` round-trip as the reachability probe,
//! folds everything through the crate's canonical checks, and best-effort
//! emits the outcome as metrics via `metrics-emit` so fleet thresholds
//! (`gang alert`) can watch it. Output: the one-liner by default (this is a
//! polling probe), `--full` for the report, `--json` for machine use.

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::{diagnostics_collect, metrics_emit, ros_interface};

use crate::{
    HealthStatus, ProbeThresholds, check_disk, check_memory, check_reachable, check_uptime,
    evaluate, format_oneliner, format_report,
};

struct Component;

fn get_u64(entries: &[diagnostics_collect::DiagnosticEntry], key: &str) -> Option<u64> {
    entries
        .iter()
        .find(|e| e.key == key)
        .and_then(|e| e.value.parse().ok())
}

const MB: u64 = 1024 * 1024;

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let thresholds = ProbeThresholds::default();

        // System stats (missing data degrades to skipped checks, not errors).
        let entries = diagnostics_collect::system_info().unwrap_or_default();
        let mem_total = get_u64(&entries, "memory_total_bytes").map(|b| b / MB);
        let mem_avail = get_u64(&entries, "memory_available_bytes").map(|b| b / MB);
        let disk_total = get_u64(&entries, "disk_total_bytes").map(|b| b / MB);
        let disk_used = match (disk_total, get_u64(&entries, "disk_available_bytes")) {
            (Some(total), Some(avail)) => Some(total.saturating_sub(avail / MB)),
            _ => None,
        };
        let uptime = get_u64(&entries, "uptime_secs");

        // Reachability: a live ROS graph listing means the robot's ROS side
        // is up and the broker path works end-to-end.
        let ros_reachable = ros_interface::list_ros().is_ok();

        let checks = vec![
            check_memory(mem_total, mem_avail, &thresholds),
            check_disk(disk_total, disk_used, &thresholds),
            check_uptime(uptime, &thresholds),
            check_reachable(ros_reachable),
        ];
        let result = evaluate(checks, "n/a", 0);

        // Best-effort metrics: the canary must never fail because the metrics
        // ring is unavailable.
        let status_value = match result.status {
            HealthStatus::Healthy => 1.0,
            HealthStatus::Degraded => 0.5,
            HealthStatus::Unhealthy | HealthStatus::Unreachable => 0.0,
        };
        let points = vec![
            metrics_emit::MetricPoint {
                name: "canary_status".to_string(),
                value: status_value,
                unit: None,
                tags: Vec::new(),
            },
            metrics_emit::MetricPoint {
                name: "canary_checks_failed".to_string(),
                value: result.failed as f64,
                unit: None,
                tags: Vec::new(),
            },
        ];
        let _ = metrics_emit::emit_batch(&points);

        if args.iter().any(|a| a == "--json") {
            serde_json::to_vec(&result).map_err(|e| e.to_string())
        } else if args.iter().any(|a| a == "--full") {
            Ok(format_report(&result).into_bytes())
        } else {
            Ok(format_oneliner(&result).into_bytes())
        }
    }
}

export!(Component);
