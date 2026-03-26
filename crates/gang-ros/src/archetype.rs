//! Network archetype detection — probes the local network to classify
//! which of the five standard archetypes the robot is deployed into.
//!
//! Used by `gang diagnose <robot>` to report the detected archetype and
//! recommend transport configuration.

use serde::{Deserialize, Serialize};

/// The five standard network archetypes Ganglion designs around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkArchetype {
    /// Flat L2, permissive DHCP, no NAT or egress controls.
    OpenWarehouse,
    /// Single consumer NAT, no inbound ports, DHCP rotation.
    NatOffice,
    /// VLAN isolation, restricted outbound ports, TLS inspection proxy.
    EnterpriseDmz,
    /// Air-gapped or physically isolated, sneakernet only.
    RegulatedFacility,
    /// Symmetric NAT, CGNAT, IP rotation, intermittent connectivity.
    MobileCgnat,
}

impl std::fmt::Display for NetworkArchetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenWarehouse => write!(f, "open-warehouse"),
            Self::NatOffice => write!(f, "nat-office"),
            Self::EnterpriseDmz => write!(f, "enterprise-dmz"),
            Self::RegulatedFacility => write!(f, "regulated-facility"),
            Self::MobileCgnat => write!(f, "mobile-cgnat"),
        }
    }
}

/// Result of network archetype detection probes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeDetectionResult {
    /// Detected archetype.
    pub archetype: NetworkArchetype,
    /// Confidence (0.0 to 1.0).
    pub confidence: f64,
    /// Individual probe results.
    pub probes: Vec<ProbeResult>,
    /// Recommended transport configuration.
    pub recommendations: Vec<String>,
}

/// Result of a single network probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub probe_name: String,
    pub success: bool,
    pub detail: String,
}

/// Run all network probes and classify the archetype.
pub fn detect_archetype() -> ArchetypeDetectionResult {
    let mut probes = Vec::new();

    // Probe 1: Check for direct internet connectivity
    let internet_probe = probe_internet_connectivity();
    probes.push(internet_probe.clone());

    // Probe 2: Check if behind NAT
    let nat_probe = probe_nat_status();
    probes.push(nat_probe.clone());

    // Probe 3: Check multicast reachability
    let multicast_probe = probe_multicast();
    probes.push(multicast_probe.clone());

    // Probe 4: Check outbound port restrictions
    let port_probe = probe_outbound_ports();
    probes.push(port_probe.clone());

    // Probe 5: Check DNS behavior
    let dns_probe = probe_dns_behavior();
    probes.push(dns_probe.clone());

    // Probe 6: Check for symmetric NAT indicators
    let symmetric_probe = probe_symmetric_nat();
    probes.push(symmetric_probe.clone());

    // Classify based on probe results
    let (archetype, confidence) = classify_archetype(&probes);

    let recommendations = generate_recommendations(&archetype, &probes);

    ArchetypeDetectionResult {
        archetype,
        confidence,
        probes,
        recommendations,
    }
}

/// Probe internet connectivity by attempting outbound connections.
fn probe_internet_connectivity() -> ProbeResult {
    // Try to resolve a well-known domain
    let output = std::process::Command::new("host")
        .args(["dns.google"])
        .output();

    match output {
        Ok(o) if o.status.success() => ProbeResult {
            probe_name: "internet_connectivity".into(),
            success: true,
            detail: "DNS resolution succeeded — outbound internet available".into(),
        },
        _ => {
            // Fallback: try ping
            let ping = std::process::Command::new("ping")
                .args(["-c", "1", "-W", "3", "8.8.8.8"])
                .output();
            match ping {
                Ok(o) if o.status.success() => ProbeResult {
                    probe_name: "internet_connectivity".into(),
                    success: true,
                    detail: "ICMP to 8.8.8.8 succeeded — outbound internet available".into(),
                },
                _ => ProbeResult {
                    probe_name: "internet_connectivity".into(),
                    success: false,
                    detail: "No outbound internet detected — possible air-gap or strict firewall"
                        .into(),
                },
            }
        }
    }
}

/// Detect NAT by comparing local IP with external perception.
fn probe_nat_status() -> ProbeResult {
    // Check default gateway
    let output = if cfg!(target_os = "linux") {
        std::process::Command::new("ip")
            .args(["route", "show", "default"])
            .output()
    } else {
        std::process::Command::new("netstat").args(["-rn"]).output()
    };

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let has_private_gw = text.contains("192.168.")
                || text.contains("10.")
                || text.contains("172.16.")
                || text.contains("172.17.")
                || text.contains("172.18.");

            if has_private_gw {
                ProbeResult {
                    probe_name: "nat_status".into(),
                    success: true,
                    detail: "Default gateway is on a private network — likely behind NAT".into(),
                }
            } else {
                ProbeResult {
                    probe_name: "nat_status".into(),
                    success: false,
                    detail: "Default gateway appears to be on a public network".into(),
                }
            }
        }
        _ => ProbeResult {
            probe_name: "nat_status".into(),
            success: false,
            detail: "Could not determine NAT status".into(),
        },
    }
}

/// Check if multicast is reachable on the local network.
fn probe_multicast() -> ProbeResult {
    // Check for multicast-capable interfaces
    let output = if cfg!(target_os = "linux") {
        std::process::Command::new("ip")
            .args(["link", "show"])
            .output()
    } else {
        std::process::Command::new("ifconfig").output()
    };

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let has_multicast = text.contains("MULTICAST");
            ProbeResult {
                probe_name: "multicast".into(),
                success: has_multicast,
                detail: if has_multicast {
                    "Multicast-capable interfaces detected".into()
                } else {
                    "No multicast-capable interfaces found".into()
                },
            }
        }
        _ => ProbeResult {
            probe_name: "multicast".into(),
            success: false,
            detail: "Could not enumerate network interfaces".into(),
        },
    }
}

/// Check outbound port accessibility beyond 80/443.
fn probe_outbound_ports() -> ProbeResult {
    // Try to connect to a known service on a non-standard port
    // Use DNS over TCP (port 53) as a quick test
    let result = std::net::TcpStream::connect_timeout(
        &"8.8.8.8:53".parse().unwrap(),
        std::time::Duration::from_secs(3),
    );

    match result {
        Ok(_) => ProbeResult {
            probe_name: "outbound_ports".into(),
            success: true,
            detail: "Non-443 outbound port (TCP 53) reachable — ports are not strictly restricted"
                .into(),
        },
        Err(_) => ProbeResult {
            probe_name: "outbound_ports".into(),
            success: false,
            detail: "Non-443 outbound port blocked — possible enterprise firewall".into(),
        },
    }
}

/// Check DNS behavior for signs of DNS interception or filtering.
fn probe_dns_behavior() -> ProbeResult {
    let output = std::process::Command::new("host")
        .args(["-t", "TXT", "dns.google"])
        .output();

    match output {
        Ok(o) if o.status.success() => ProbeResult {
            probe_name: "dns_behavior".into(),
            success: true,
            detail: "DNS TXT queries succeed — no apparent DNS filtering".into(),
        },
        _ => ProbeResult {
            probe_name: "dns_behavior".into(),
            success: false,
            detail: "DNS TXT queries failed — possible DNS filtering or interception".into(),
        },
    }
}

/// Check for symmetric NAT indicators (CGNAT address ranges).
fn probe_symmetric_nat() -> ProbeResult {
    let output = if cfg!(target_os = "linux") {
        std::process::Command::new("ip")
            .args(["addr", "show"])
            .output()
    } else {
        std::process::Command::new("ifconfig").output()
    };

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            // CGNAT range: 100.64.0.0/10
            let has_cgnat = text.contains("100.64.")
                || text.contains("100.65.")
                || text.contains("100.66.")
                || text.contains("100.67.")
                || text.contains("100.68.")
                || text.contains("100.96.")
                || text.contains("100.127.");

            if has_cgnat {
                ProbeResult {
                    probe_name: "symmetric_nat".into(),
                    success: true,
                    detail: "CGNAT address range (100.64.0.0/10) detected — likely carrier NAT"
                        .into(),
                }
            } else {
                ProbeResult {
                    probe_name: "symmetric_nat".into(),
                    success: false,
                    detail: "No CGNAT address ranges detected".into(),
                }
            }
        }
        _ => ProbeResult {
            probe_name: "symmetric_nat".into(),
            success: false,
            detail: "Could not check for CGNAT addresses".into(),
        },
    }
}

/// Classify the archetype based on probe results.
fn classify_archetype(probes: &[ProbeResult]) -> (NetworkArchetype, f64) {
    let internet = probes
        .iter()
        .find(|p| p.probe_name == "internet_connectivity")
        .map(|p| p.success)
        .unwrap_or(false);
    let nat = probes
        .iter()
        .find(|p| p.probe_name == "nat_status")
        .map(|p| p.success)
        .unwrap_or(false);
    let multicast = probes
        .iter()
        .find(|p| p.probe_name == "multicast")
        .map(|p| p.success)
        .unwrap_or(false);
    let ports_open = probes
        .iter()
        .find(|p| p.probe_name == "outbound_ports")
        .map(|p| p.success)
        .unwrap_or(false);
    let cgnat = probes
        .iter()
        .find(|p| p.probe_name == "symmetric_nat")
        .map(|p| p.success)
        .unwrap_or(false);

    // Classification logic
    if !internet {
        return (NetworkArchetype::RegulatedFacility, 0.8);
    }

    if cgnat {
        return (NetworkArchetype::MobileCgnat, 0.85);
    }

    if !ports_open && nat {
        return (NetworkArchetype::EnterpriseDmz, 0.8);
    }

    if nat && multicast {
        return (NetworkArchetype::NatOffice, 0.75);
    }

    if nat {
        return (NetworkArchetype::NatOffice, 0.7);
    }

    if multicast && ports_open {
        return (NetworkArchetype::OpenWarehouse, 0.85);
    }

    // Default: assume NAT'd office if we can't tell
    (NetworkArchetype::NatOffice, 0.5)
}

/// Generate transport recommendations based on the detected archetype.
fn generate_recommendations(archetype: &NetworkArchetype, _probes: &[ProbeResult]) -> Vec<String> {
    match archetype {
        NetworkArchetype::OpenWarehouse => vec![
            "Direct QUIC connection recommended — lowest latency".into(),
            "No relay needed for this network topology".into(),
            "Multicast discovery available for peer finding".into(),
        ],
        NetworkArchetype::NatOffice => vec![
            "Configure a relay server for initial connectivity".into(),
            "DCUtR hole-punch should succeed with endpoint-independent NAT".into(),
            "Connection will upgrade to direct QUIC after hole-punch".into(),
        ],
        NetworkArchetype::EnterpriseDmz => vec![
            "Relay on TCP 443 is required — QUIC (UDP) likely blocked".into(),
            "Configure relay to listen on port 443".into(),
            "DCUtR will likely fail — plan for relay-only operation".into(),
            "Expect +5-20ms additional latency from TLS inspection".into(),
        ],
        NetworkArchetype::RegulatedFacility => vec![
            "No network connectivity detected — use offline signed bundles".into(),
            "Pre-sign capabilities on an external machine".into(),
            "Transfer via USB or approved sneakernet process".into(),
        ],
        NetworkArchetype::MobileCgnat => vec![
            "Relay-only connectivity — symmetric NAT defeats hole-punching".into(),
            "Configure aggressive reconnection logic".into(),
            "Expect variable latency (50ms+ base) and intermittent drops".into(),
            "Use chunked transfers for large payloads".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_no_internet() {
        let probes = vec![
            ProbeResult {
                probe_name: "internet_connectivity".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "nat_status".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "multicast".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "outbound_ports".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "symmetric_nat".into(),
                success: false,
                detail: String::new(),
            },
        ];
        let (archetype, _) = classify_archetype(&probes);
        assert_eq!(archetype, NetworkArchetype::RegulatedFacility);
    }

    #[test]
    fn classify_cgnat() {
        let probes = vec![
            ProbeResult {
                probe_name: "internet_connectivity".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "nat_status".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "multicast".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "outbound_ports".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "symmetric_nat".into(),
                success: true,
                detail: String::new(),
            },
        ];
        let (archetype, _) = classify_archetype(&probes);
        assert_eq!(archetype, NetworkArchetype::MobileCgnat);
    }

    #[test]
    fn classify_enterprise_dmz() {
        let probes = vec![
            ProbeResult {
                probe_name: "internet_connectivity".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "nat_status".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "multicast".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "outbound_ports".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "symmetric_nat".into(),
                success: false,
                detail: String::new(),
            },
        ];
        let (archetype, _) = classify_archetype(&probes);
        assert_eq!(archetype, NetworkArchetype::EnterpriseDmz);
    }

    #[test]
    fn classify_open_warehouse() {
        let probes = vec![
            ProbeResult {
                probe_name: "internet_connectivity".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "nat_status".into(),
                success: false,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "multicast".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "outbound_ports".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "symmetric_nat".into(),
                success: false,
                detail: String::new(),
            },
        ];
        let (archetype, _) = classify_archetype(&probes);
        assert_eq!(archetype, NetworkArchetype::OpenWarehouse);
    }

    #[test]
    fn classify_nat_office() {
        let probes = vec![
            ProbeResult {
                probe_name: "internet_connectivity".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "nat_status".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "multicast".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "outbound_ports".into(),
                success: true,
                detail: String::new(),
            },
            ProbeResult {
                probe_name: "symmetric_nat".into(),
                success: false,
                detail: String::new(),
            },
        ];
        let (archetype, _) = classify_archetype(&probes);
        assert_eq!(archetype, NetworkArchetype::NatOffice);
    }

    #[test]
    fn archetype_display() {
        assert_eq!(
            NetworkArchetype::OpenWarehouse.to_string(),
            "open-warehouse"
        );
        assert_eq!(NetworkArchetype::MobileCgnat.to_string(), "mobile-cgnat");
        assert_eq!(
            NetworkArchetype::EnterpriseDmz.to_string(),
            "enterprise-dmz"
        );
    }

    #[test]
    fn recommendations_nonempty() {
        for archetype in [
            NetworkArchetype::OpenWarehouse,
            NetworkArchetype::NatOffice,
            NetworkArchetype::EnterpriseDmz,
            NetworkArchetype::RegulatedFacility,
            NetworkArchetype::MobileCgnat,
        ] {
            let recs = generate_recommendations(&archetype, &[]);
            assert!(!recs.is_empty(), "no recommendations for {archetype}");
        }
    }

    #[test]
    fn detect_archetype_runs() {
        // This runs the actual probes — result depends on host network
        let result = detect_archetype();
        assert!(!result.probes.is_empty());
        assert!(result.confidence > 0.0);
        assert!(!result.recommendations.is_empty());
    }
}
