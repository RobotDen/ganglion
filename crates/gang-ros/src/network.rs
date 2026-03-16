use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use gang_core::broker::{BrokerOperation, CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::error::BrokerError;

/// Network probe broker — structured network probing primitives.
///
/// Provides ping, DNS lookup, TCP port check, and traceroute operations
/// for use by network diagnostics and archetype detection capabilities.
pub struct NetworkProbeBroker;

impl NetworkProbeBroker {
    pub fn new() -> Self {
        Self
    }
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

fn do_ping(host: &str, count: u32) -> PingResult {
    // Use a TCP connect to port 80 as a userspace "ping" (no raw sockets needed).
    let addr_str = format!("{host}:80");
    let mut received = 0u32;
    let mut total_rtt = 0.0f64;

    for _ in 0..count {
        let start = Instant::now();
        if let Ok(addrs) = addr_str.to_socket_addrs() {
            for addr in addrs {
                match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
                    Ok(_) => {
                        received += 1;
                        total_rtt += start.elapsed().as_secs_f64() * 1000.0;
                        break;
                    }
                    Err(_) => {}
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
                match std::net::TcpStream::connect_timeout(
                    &addr,
                    Duration::from_secs(timeout_secs),
                ) {
                    Ok(_) => {
                        found = true;
                        lat = start.elapsed().as_secs_f64() * 1000.0;
                        break;
                    }
                    Err(_) => {}
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
                let result = do_ping(&host, count);
                let data = serde_json::to_vec(&result).map_err(|e| BrokerError::Unavailable {
                    broker: "network-probe".into(),
                    reason: e.to_string(),
                })?;
                let bytes_out = data.len() as u64;
                Ok(CapabilityResponse {
                    success: result.reachable,
                    data,
                    error: if !result.reachable {
                        Some(format!("host {} unreachable", host))
                    } else {
                        None
                    },
                    bytes_in: 0,
                    bytes_out,
                })
            }
            BrokerOperation::NetDnsLookup {
                hostname,
                record_type,
            } => {
                let result = do_dns_lookup(&hostname, &record_type);
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
                let result = do_port_check(&host, port, timeout_secs);
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
                let result = do_traceroute(&host, max_hops);
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

    #[tokio::test]
    async fn broker_dns_lookup() {
        let broker = NetworkProbeBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::NetDnsLookup {
                hostname: "localhost".into(),
                record_type: "A".into(),
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(resp.success);
        let result: DnsResult = serde_json::from_slice(&resp.data).unwrap();
        assert!(!result.answers.is_empty());
    }

    #[tokio::test]
    async fn broker_port_check() {
        let broker = NetworkProbeBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::NetPortCheck {
                host: "127.0.0.1".into(),
                port: 1,
                timeout_secs: 1,
            },
        };
        let resp = broker.handle_request(req).await.unwrap();
        assert!(!resp.success); // Port 1 should be closed
    }

    #[tokio::test]
    async fn broker_rejects_unknown_op() {
        let broker = NetworkProbeBroker::new();
        let req = CapabilityRequest {
            capability_group: "ganglion:network/probe".into(),
            operation: BrokerOperation::SystemInfo,
        };
        assert!(broker.handle_request(req).await.is_err());
    }
}
