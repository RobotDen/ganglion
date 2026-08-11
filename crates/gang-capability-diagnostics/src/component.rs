//! WASM component entry point (wasm32 builds only).
//!
//! Bridges the `ganglion-capability` world's `run` export to this crate's
//! canonical logic: gathers real system data through the
//! `diagnostics-collect` host import, folds it into a [`DiagnosticReport`],
//! and returns the formatted report (or JSON with `--json`).
//!
//! The host flattens the broker's `SystemInfo` struct into `diagnostic-entry`
//! key/value pairs, so the keys seen here are the struct's field names
//! (`hostname`, `os`, `os_version`, `arch`, `uptime_secs`,
//! `memory_total_bytes`, `memory_available_bytes`, `disk_total_bytes`,
//! `disk_available_bytes`, …). Timestamps are reported as `n/a`: the host's
//! WASI clock set is deny-by-default, and the report's freshness is implied by
//! the invocation itself.

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::diagnostics_collect;

use crate::{DiagnosticReport, DiskEntry};

struct Component;

/// Find an entry's value by key.
fn get<'a>(entries: &'a [diagnostics_collect::DiagnosticEntry], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.value.as_str())
}

/// Parse an entry as u64, if present.
fn get_u64(entries: &[diagnostics_collect::DiagnosticEntry], key: &str) -> Option<u64> {
    get(entries, key).and_then(|v| v.parse().ok())
}

const MB: u64 = 1024 * 1024;

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let entries = diagnostics_collect::system_info()?;

        let os = match (get(&entries, "os"), get(&entries, "os_version")) {
            (Some(os), Some(ver)) if !ver.is_empty() => format!("{os} {ver}"),
            (Some(os), _) => os.to_string(),
            _ => "unknown".to_string(),
        };

        let disk_total = get_u64(&entries, "disk_total_bytes");
        let disk_avail = get_u64(&entries, "disk_available_bytes");
        let disk_usage = match (disk_total, disk_avail) {
            (Some(total), Some(avail)) if total > 0 => vec![DiskEntry {
                mount: "/".to_string(),
                total_mb: total / MB,
                used_mb: total.saturating_sub(avail) / MB,
            }],
            _ => Vec::new(),
        };

        let report = DiagnosticReport {
            timestamp: "n/a".to_string(),
            hostname: get(&entries, "hostname").unwrap_or("unknown").to_string(),
            os,
            arch: get(&entries, "arch").unwrap_or("unknown").to_string(),
            uptime_secs: get_u64(&entries, "uptime_secs"),
            memory_total_mb: get_u64(&entries, "memory_total_bytes").map(|b| b / MB),
            memory_available_mb: get_u64(&entries, "memory_available_bytes").map(|b| b / MB),
            disk_usage,
        };

        if args.iter().any(|a| a == "--json") {
            serde_json::to_vec(&report).map_err(|e| e.to_string())
        } else {
            Ok(crate::format_report(&report).into_bytes())
        }
    }
}

export!(Component);
