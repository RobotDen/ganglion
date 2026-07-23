use std::net::{IpAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Network probe broker — structured network probing primitives.
///
/// Provides ping, DNS lookup, TCP port check, and traceroute operations
/// for use by network diagnostics and archetype detection capabilities.
///
/// Every probe target is checked against a configured host/CIDR allowlist,
/// and a set of sensitive ranges (loopback, link-local/metadata, IPv6 ULA)
/// is blocked unconditionally regardless of the allowlist to prevent SSRF
/// and cloud-metadata exfiltration.
pub struct NetworkProbeBroker {
    /// Allowlist of probe targets. Each entry is either a CIDR (contains `/`,
    /// matched against the target's IP addresses) or a glob pattern matched
    /// against the host string. The special entry `**` allows any host (still
    /// subject to the unconditional blocked ranges). An empty allowlist denies
    /// every target (default-deny).
    allowed_hosts: Vec<String>,
}

impl Default for NetworkProbeBroker {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl NetworkProbeBroker {
    /// Create a broker with the given host/CIDR allowlist.
    pub fn new(allowed_hosts: Vec<String>) -> Self {
        Self { allowed_hosts }
    }

    /// Convenience constructor allowing any host (still subject to the
    /// unconditionally-blocked ranges).
    pub fn allow_all() -> Self {
        Self {
            allowed_hosts: vec!["**".into()],
        }
    }
}

/// Return true if `ip` falls in a range that is blocked for probing
/// unconditionally: IPv4 loopback (127.0.0.0/8), IPv4 link-local
/// (169.254.0.0/16, which includes the 169.254.169.254 cloud metadata
/// endpoint), IPv6 loopback (::1), and IPv6 unique-local (fc00::/7).
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            // fc00::/7: top 7 bits are 1111110.
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Return true if `ip` is contained in the CIDR block `cidr` (e.g.
/// "10.0.0.0/8" or "fd00::/8"). Returns false on malformed input or an
/// address-family mismatch.
fn ip_in_cidr(ip: &IpAddr, cidr: &str) -> bool {
    let Some((addr, prefix)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u32>() else {
        return false;
    };
    match (ip, addr.parse::<IpAddr>()) {
        (IpAddr::V4(ip), Ok(IpAddr::V4(net))) => {
            if prefix > 32 {
                return false;
            }
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(*ip) & mask) == (u32::from(net) & mask)
        }
        (IpAddr::V6(ip), Ok(IpAddr::V6(net))) => {
            if prefix > 128 {
                return false;
            }
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(*ip) & mask) == (u128::from(net) & mask)
        }
        _ => false,
    }
}

/// Validate a probe target against the blocked ranges and the allowlist.
///
/// This resolves hostnames (blocking DNS), so it must run inside a
/// `spawn_blocking` context, never directly on the async executor.
///
/// Order of enforcement:
/// 1. The blocked ranges are enforced unconditionally against every resolved
///    IP (defeating DNS-rebinding to loopback/metadata).
/// 2. The target must then match the configured allowlist.
fn check_probe_target(allowed_hosts: &[String], host: &str) -> Result<(), BrokerError> {
    // Candidate IPs: an IP literal resolves to itself; otherwise resolve DNS.
    let ips: Vec<IpAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![ip]
    } else {
        (host, 0u16)
            .to_socket_addrs()
            .map(|it| it.map(|s| s.ip()).collect())
            .unwrap_or_default()
    };

    // 1. Unconditional block ranges.
    if let Some(blocked) = ips.iter().find(|ip| is_blocked_ip(ip)) {
        return Err(BrokerError::AccessDenied {
            broker: "network-probe".into(),
            resource: host.into(),
            reason: format!(
                "target resolves to blocked range ({blocked}): loopback/link-local/metadata/ULA \
                 probing is denied"
            ),
        });
    }

    // 2. Allowlist.
    let allowed = allowed_hosts.iter().any(|entry| {
        if entry == "**" {
            true
        } else if entry.contains('/') {
            ips.iter().any(|ip| ip_in_cidr(ip, entry))
        } else {
            glob_match::glob_match(entry, host)
        }
    });

    if !allowed {
        return Err(BrokerError::AccessDenied {
            broker: "network-probe".into(),
            resource: host.into(),
            reason: "host not permitted by probe allowlist".into(),
        });
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResult {
    pub host: String,
    pub reachable: bool,
    pub rtt_ms: f64,
    pub packets_sent: u32,
    pub packets_received: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsResult {
    pub hostname: String,
    pub record_type: String,
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortResult {
    pub host: String,
    pub port: u16,
    pub open: bool,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracerouteHop {
    pub hop: u32,
    pub address: String,
    pub rtt_ms: f64,
}

/// Run a blocking probe closure on the blocking thread pool, flattening the
/// join error and the closure's own `Result` into a single `BrokerError`.
async fn spawn_probe<T, F>(f: F) -> Result<T, BrokerError>
where
    F: FnOnce() -> Result<T, BrokerError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(inner) => inner,
        Err(e) => Err(BrokerError::Unavailable {
            broker: "network-probe".into(),
            reason: format!("probe task failed: {e}"),
        }),
    }
}

fn do_ping(host: &str, count: u32) -> PingResult {
    // Use a TCP connect to port 80 as a userspace "ping" (no raw sockets needed).
    let addr_str = format!("{host}:80");
    let mut received = 0u32;
    let mut total_rtt = 0.0f64;

    for _ in 0..count {
        let start = Instant::now();
        if let Ok(addrs) = addr_str.to_socket_addrs() {
            for addr in addrs {
                if std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok() {
                    received += 1;
                    total_rtt += start.elapsed().as_secs_f64() * 1000.0;
                    break;
                }
            }
        }
    }

    PingResult {
        host: host.to_string(),
        reachable: received > 0,
        rtt_ms: if received > 0 {
            total_rtt / received as f64
        } else {
            0.0
        },
        packets_sent: count,
        packets_received: received,
    }
}

fn do_dns_lookup(hostname: &str, record_type: &str) -> DnsResult {
    // Standard library DNS resolution (A/AAAA only — no TXT/MX without a DNS library).
    let answers = match record_type {
        "A" | "AAAA" | "a" | "aaaa" => {
            let addr_str = format!("{hostname}:0");
            match addr_str.to_socket_addrs() {
                Ok(addrs) => addrs.map(|a| a.ip().to_string()).collect(),
                Err(_) => vec![],
            }
        }
        _ => {
            // For other record types, return empty — would need trust-dns or similar.
            vec![]
        }
    };

    DnsResult {
        hostname: hostname.to_string(),
        record_type: record_type.to_string(),
        answers,
    }
}

fn do_port_check(host: &str, port: u16, timeout_secs: u64) -> PortResult {
    let addr_str = format!("{host}:{port}");
    let start = Instant::now();

    let (open, latency_ms) = match addr_str.to_socket_addrs() {
        Ok(addrs) => {
            let mut found = false;
            let mut lat = 0.0;
            for addr in addrs {
                if std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(timeout_secs))
                    .is_ok()
                {
                    found = true;
                    lat = start.elapsed().as_secs_f64() * 1000.0;
                    break;
                }
            }
            (found, lat)
        }
        Err(_) => (false, 0.0),
    };

    PortResult {
        host: host.to_string(),
        port,
        open,
        latency_ms,
    }
}

fn do_traceroute(host: &str, max_hops: u32) -> Vec<TracerouteHop> {
    // Traceroute requires raw sockets or elevated privileges.
    // Return a stub with the destination as the only hop for userspace contexts.
    let ping = do_ping(host, 1);
    if ping.reachable {
        vec![TracerouteHop {
            hop: 1,
            address: host.to_string(),
            rtt_ms: ping.rtt_ms,
        }]
    } else {
        vec![TracerouteHop {
            hop: max_hops,
            address: "*".to_string(),
            rtt_ms: 0.0,
        }]
    }
}

#[async_trait]
impl CapabilityBroker for NetworkProbeBroker {
    async fn handle_request(
        &self,
        req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        match req.operation {
            BrokerOperation::NetPing { host, count } => {
                let allowed = self.allowed_hosts.clone();
                // Allowlist/block enforcement AND the blocking TCP probe run
                // off the async executor (CODE-11).
                let result = spawn_probe(move || {
                    check_probe_target(&allowed, &host)?;
                    Ok(do_ping(&host, count))
                })
                .await?;

                let data = serde_json::to_vec(&result).map_err(|e| BrokerError::Unavailable {
                    broker: "network-probe".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: result.reachable,
                    error: if !result.reachable {
                        Some(format!("host {} unreachable", result.host))
                    } else {
                        None
                    },
                    data,
                    bytes_in: 0,
                    bytes_out,
                })
            }
            BrokerOperation::NetDnsLookup {
                hostname,
                record_type,
            } => {
                let allowed = self.allowed_hosts.clone();
                let result = spawn_probe(move || {
                    check_probe_target(&allowed, &hostname)?;
                    Ok(do_dns_lookup(&hostname, &record_type))
                })
                .await?;

                let data = serde_json::to_vec(&result).map_err(|e| BrokerError::Unavailable {
                    broker: "network-probe".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: !result.answers.is_empty(),
                    data,
                    error: None,
                    bytes_in: 0,
                    bytes_out,
                })
            }
            BrokerOperation::NetPortCheck {
                host,
                port,
                timeout_secs,
            } => {
                let allowed = self.allowed_hosts.clone();
                let result = spawn_probe(move || {
                    check_probe_target(&allowed, &host)?;
                    Ok(do_port_check(&host, port, timeout_secs))
                })
                .await?;

                let data = serde_json::to_vec(&result).map_err(|e| BrokerError::Unavailable {
                    broker: "network-probe".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: result.open,
                    data,
                    error: None,
                    bytes_in: 0,
                    bytes_out,
                })
            }
            BrokerOperation::NetTraceroute { host, max_hops } => {
                let allowed = self.allowed_hosts.clone();
                let result = spawn_probe(move || {
                    check_probe_target(&allowed, &host)?;
                    Ok(do_traceroute(&host, max_hops))
                })
                .await?;

                let data = serde_json::to_vec(&result).map_err(|e| BrokerError::Unavailable {
                    broker: "network-probe".into(),
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
                broker: "network-probe".into(),
                resource: format!("{:?}", req.operation),
                reason: "operation not supported by network probe broker".into(),
            }),
        }
    }

    fn capability_group(&self) -> &str {
        "ganglion:network/probe"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_lookup_localhost() {
        let result = do_dns_lookup("localhost", "A");
        assert_eq!(result.hostname, "localhost");
        assert!(!result.answers.is_empty());
    }

    #[test]
    fn dns_lookup_unsupported_type() {
        let result = do_dns_lookup("example.com", "TXT");
        // TXT not supported via std — returns empty
        assert!(result.answers.is_empty());
    }

    #[test]
    fn port_check_unreachable() {
        // Port 1 on localhost is almost certainly closed
        let result = do_port_check("127.0.0.1", 1, 1);
        assert!(!result.open);
    }

    #[test]
    fn traceroute_stub() {
        let hops = do_traceroute("127.0.0.1", 30);
        assert!(!hops.is_empty());
    }

    // --- SEC-12: blocked-range enforcement (unit, no network) ---

    #[test]
    fn is_blocked_ip_ranges() {
        assert!(is_blocked_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"127.255.255.254".parse().unwrap()));
        assert!(is_blocked_ip(&"169.254.0.1".parse().unwrap()));
        // Cloud metadata endpoint.
        assert!(is_blocked_ip(&"169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip(&"::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fc00::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fd12:3456::1".parse().unwrap()));
        // Not blocked.
        assert!(!is_blocked_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_blocked_ip(&"10.0.0.1".parse().unwrap()));
        assert!(!is_blocked_ip(&"2001:4860:4860::8888".parse().unwrap()));
    }

    #[test]
    fn ip_in_cidr_matching() {
        assert!(ip_in_cidr(&"10.1.2.3".parse().unwrap(), "10.0.0.0/8"));
        assert!(!ip_in_cidr(&"11.1.2.3".parse().unwrap(), "10.0.0.0/8"));
        assert!(ip_in_cidr(
            &"192.168.1.5".parse().unwrap(),
            "192.168.0.0/16"
        ));
        assert!(!ip_in_cidr(
            &"192.169.1.5".parse().unwrap(),
            "192.168.0.0/16"
        ));
        assert!(ip_in_cidr(&"fd00::5".parse().unwrap(), "fd00::/8"));
        // Family mismatch and malformed inputs.
        assert!(!ip_in_cidr(&"10.0.0.1".parse().unwrap(), "fd00::/8"));
        assert!(!ip_in_cidr(&"10.0.0.1".parse().unwrap(), "garbage"));
    }

    #[test]
    fn check_probe_target_blocks_ranges_unconditionally() {
        // Even with an allow-all list, blocked ranges are denied.
        for host in ["127.0.0.1", "169.254.169.254", "::1", "fc00::1"] {
            let err = check_probe_target(&["**".into()], host).unwrap_err();
            assert!(
                matches!(err, BrokerError::AccessDenied { .. }),
                "{host} should be blocked"
            );
        }
    }

    #[test]
    fn check_probe_target_enforces_allowlist() {
        // Not in allowlist -> denied.
        let err = check_probe_target(&[], "8.8.8.8").unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));

        // Exact/glob host allow.
        assert!(check_probe_target(&["8.8.8.8".into()], "8.8.8.8").is_ok());
        // CIDR allow.
        assert!(check_probe_target(&["8.8.0.0/16".into()], "8.8.8.8").is_ok());
        // Allow-all.
        assert!(check_probe_target(&["**".into()], "8.8.8.8").is_ok());
    }

    #[tokio::test]
    async fn broker_ping_blocks_loopback() {
        // SEC-12: loopback is blocked even under allow_all, and the denial
        // short-circuits before any network probe.
        let broker = NetworkProbeBroker::allow_all();
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::NetPing {
                host: "127.0.0.1".into(),
                count: 1,
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));
    }

    #[tokio::test]
    async fn broker_dns_denies_non_allowlisted() {
        // Empty allowlist denies everything (default-deny).
        let broker = NetworkProbeBroker::new(vec![]);
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::NetDnsLookup {
                hostname: "8.8.8.8".into(),
                record_type: "A".into(),
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));
    }

    #[tokio::test]
    async fn broker_port_check_blocks_loopback() {
        let broker = NetworkProbeBroker::allow_all();
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::NetPortCheck {
                host: "127.0.0.1".into(),
                port: 1,
                timeout_secs: 1,
            },
        };
        let err = broker.handle_request(req).await.unwrap_err();
        assert!(matches!(err, BrokerError::AccessDenied { .. }));
    }

    #[tokio::test]
    async fn broker_rejects_unknown_op() {
        let broker = NetworkProbeBroker::allow_all();
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::SystemInfo,
        };
        assert!(broker.handle_request(req).await.is_err());
    }
}
