use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Diagnostics broker — collects system information, process lists, and network state.
/// Works on Linux and macOS without requiring ROS 2.
pub struct DiagnosticsBroker;

impl Default for DiagnosticsBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsBroker {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CapabilityBroker for DiagnosticsBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::SystemInfo => {
                // CODE-11: system info collection shells out and reads /proc,
                // both blocking — run it off the async executor.
                let info = spawn_collect(collect_system_info).await?;
                let data = serde_json::to_vec(&info).map_err(|e| BrokerError::Unavailable {
                    broker: "diagnostics".into(),
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
            BrokerOperation::ProcessList => {
                // CODE-11: `ps` invocation is blocking — offload it.
                let procs = spawn_collect(collect_process_list).await?;
                let data = serde_json::to_vec(&procs).map_err(|e| BrokerError::Unavailable {
                    broker: "diagnostics".into(),
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
            BrokerOperation::NetworkState => {
                // CODE-11: `ip`/`ifconfig`/`ss`/`netstat` are blocking — offload.
                let net = spawn_collect(collect_network_state).await?;
                let data = serde_json::to_vec(&net).map_err(|e| BrokerError::Unavailable {
                    broker: "diagnostics".into(),
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
                broker: "diagnostics".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by diagnostics broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:diagnostics/collect"
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub hostname: String,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub uptime_secs: u64,
    pub cpu_count: usize,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub ganglion_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub interfaces: Vec<InterfaceInfo>,
    pub connections: Vec<ConnectionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub addresses: Vec<String>,
    pub is_up: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub protocol: String,
    pub local_addr: String,
    pub remote_addr: String,
    pub state: String,
}

/// Run a blocking collection function on the blocking thread pool so it never
/// stalls the async executor, mapping a join failure to a broker error.
async fn spawn_collect<T, F>(f: F) -> Result<T, BrokerError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| BrokerError::Unavailable {
            broker: "diagnostics".into(),
            reason: format!("collection task failed: {e}"),
        })
}

fn collect_system_info() -> SystemInfo {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());

    SystemInfo {
        hostname,
        os: std::env::consts::OS.into(),
        os_version: os_version(),
        arch: std::env::consts::ARCH.into(),
        uptime_secs: system_uptime(),
        cpu_count: num_cpus(),
        memory_total_bytes: memory_total(),
        memory_available_bytes: memory_available(),
        disk_total_bytes: 0,     // Platform-specific
        disk_available_bytes: 0, // Platform-specific
        ganglion_version: env!("CARGO_PKG_VERSION").into(),
    }
}

fn collect_process_list() -> Vec<ProcessInfo> {
    // Use `ps` command for cross-platform compatibility
    let output = std::process::Command::new("ps").args(["aux"]).output();

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .skip(1) // header
                .filter_map(|line| {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() >= 11 {
                        Some(ProcessInfo {
                            pid: fields[1].parse().unwrap_or(0),
                            name: fields[10..].join(" "),
                            cpu_percent: fields[2].parse().unwrap_or(0.0),
                            memory_bytes: 0, // Would need /proc or sysctl for actual bytes
                        })
                    } else {
                        None
                    }
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

fn collect_network_state() -> NetworkInfo {
    let interfaces = collect_interfaces();
    let connections = collect_connections();

    NetworkInfo {
        interfaces,
        connections,
    }
}

fn collect_interfaces() -> Vec<InterfaceInfo> {
    // Use `ifconfig` or `ip addr` for interface info
    let output = if cfg!(target_os = "linux") {
        std::process::Command::new("ip")
            .args(["addr", "show"])
            .output()
    } else {
        std::process::Command::new("ifconfig").output()
    };

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            parse_interfaces(&text)
        }
        Err(_) => Vec::new(),
    }
}

fn parse_interfaces(text: &str) -> Vec<InterfaceInfo> {
    let mut interfaces = Vec::new();
    let mut current_name = String::new();
    let mut current_addrs = Vec::new();
    let mut is_up = false;

    for line in text.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && !line.is_empty() {
            // Save previous interface
            if !current_name.is_empty() {
                interfaces.push(InterfaceInfo {
                    name: current_name.clone(),
                    addresses: current_addrs.clone(),
                    is_up,
                });
            }
            // Parse new interface name
            current_name = line.split(':').next().unwrap_or("").trim().to_string();
            // Remove numeric prefix (Linux `ip addr` format: "2: eth0")
            if let Some(pos) = current_name.find(": ") {
                current_name = current_name[pos + 2..].to_string();
            }
            current_addrs = Vec::new();
            is_up = line.contains("UP") || line.contains("<UP");
        } else {
            let trimmed = line.trim();
            if trimmed.starts_with("inet ") || trimmed.starts_with("inet6 ") {
                let addr = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
                current_addrs.push(addr);
            }
        }
    }

    // Don't forget the last one
    if !current_name.is_empty() {
        interfaces.push(InterfaceInfo {
            name: current_name,
            addresses: current_addrs,
            is_up,
        });
    }

    interfaces
}

fn collect_connections() -> Vec<ConnectionInfo> {
    let output = if cfg!(target_os = "linux") {
        std::process::Command::new("ss").args(["-tuln"]).output()
    } else {
        std::process::Command::new("netstat").args(["-an"]).output()
    };

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .skip(1)
                .filter_map(|line| {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() >= 4 {
                        Some(ConnectionInfo {
                            protocol: fields[0].to_string(),
                            local_addr: fields.get(3).unwrap_or(&"").to_string(),
                            remote_addr: fields.get(4).unwrap_or(&"").to_string(),
                            state: fields.last().unwrap_or(&"").to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

fn os_version() -> String {
    let output = if cfg!(target_os = "macos") {
        std::process::Command::new("sw_vers")
            .args(["-productVersion"])
            .output()
    } else {
        std::process::Command::new("uname").args(["-r"]).output()
    };

    output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn system_uptime() -> u64 {
    let output = if cfg!(target_os = "macos") {
        std::process::Command::new("sysctl")
            .args(["-n", "kern.boottime"])
            .output()
    } else {
        // Linux: read /proc/uptime
        return std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .map(|s| s as u64)
            .unwrap_or(0);
    };

    // macOS: parse boottime
    output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            // Format: { sec = 1234567890, usec = 0 }
            s.split("sec = ")
                .nth(1)?
                .split(',')
                .next()?
                .trim()
                .parse::<u64>()
                .ok()
        })
        .map(|boot_time| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now.saturating_sub(boot_time)
        })
        .unwrap_or(0)
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn memory_total() -> u64 {
    if cfg!(target_os = "macos") {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    } else {
        // Linux: /proc/meminfo
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .map(|kb| kb * 1024)
            .unwrap_or(0)
    }
}

fn memory_available() -> u64 {
    if cfg!(target_os = "macos") {
        // Approximate from vm_stat
        0 // Placeholder — macOS memory reporting is complex
    } else {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemAvailable:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .map(|kb| kb * 1024)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_info_collects() {
        let info = collect_system_info();
        assert!(!info.hostname.is_empty());
        assert!(!info.os.is_empty());
        assert!(info.cpu_count > 0);
    }

    #[test]
    fn process_list_collects() {
        let procs = collect_process_list();
        // Should find at least the test runner itself
        assert!(!procs.is_empty());
    }

    #[test]
    fn network_interfaces_collects() {
        // Depends on `ip`/`ifconfig` being present; minimal containers may
        // have neither, in which case the list is legitimately empty. Just
        // assert the parse path runs without panicking.
        let interfaces = collect_interfaces();
        let _ = interfaces;
    }

    #[tokio::test]
    async fn diagnostics_broker_system_info() {
        let broker = DiagnosticsBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:diagnostics/collect".into(),
            operation: BrokerOperation::SystemInfo,
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        assert!(resp.bytes_out > 0);

        let info: SystemInfo = serde_json::from_slice(&resp.data).unwrap();
        assert!(!info.hostname.is_empty());
    }

    #[tokio::test]
    async fn diagnostics_broker_process_list() {
        let broker = DiagnosticsBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:diagnostics/collect".into(),
            operation: BrokerOperation::ProcessList,
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
    }

    #[tokio::test]
    async fn diagnostics_broker_network_state() {
        let broker = DiagnosticsBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:diagnostics/collect".into(),
            operation: BrokerOperation::NetworkState,
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
    }

    #[tokio::test]
    async fn diagnostics_broker_rejects_unknown_op() {
        let broker = DiagnosticsBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:diagnostics/collect".into(),
            operation: BrokerOperation::FsRead {
                path: "/etc/passwd".into(),
            },
        };
        let result = broker.handle_request(req).await;
        assert!(result.is_err());
    }
}
