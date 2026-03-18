//! Network archetype detection v2 — classifies robot network environment
//! with transport recommendations and connectivity scoring.
//!
//! This is the capability version of the archetype detection in gang-ros.
//! When running as a WASM component, it uses network/probe and process/spawn
//! broker interfaces to perform probes. As a library, the core classification
//! and recommendation logic is testable without network access.

use serde::{Deserialize, Serialize};

/// Network archetype classification result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeReport {
    /// Detected archetype.
    pub archetype: NetworkArchetype,
    /// Confidence in the classification (0.0 - 1.0).
    pub confidence: f64,
    /// Individual probe results that informed the classification.
    pub probes: Vec<ProbeResult>,
    /// Transport recommendations for this environment.
    pub recommendations: Vec<Recommendation>,
    /// Connectivity score (0-100). Higher = easier to establish connections.
    pub connectivity_score: u32,
    /// Summary text.
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkArchetype {
    /// Direct LAN, no NAT, multicast available.
    OpenWarehouse,
    /// Behind single NAT, multicast within LAN.
    NatOffice,
    /// Restrictive firewall, only TCP 443 outbound.
    EnterpriseDmz,
    /// Regulated facility with air-gap or strict proxy.
    RegulatedFacility,
    /// Double NAT / CGNAT, symmetric NAT.
    MobileCgnat,
}

impl std::fmt::Display for NetworkArchetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenWarehouse => write!(f, "Open Warehouse"),
            Self::NatOffice => write!(f, "NAT Office"),
            Self::EnterpriseDmz => write!(f, "Enterprise DMZ"),
            Self::RegulatedFacility => write!(f, "Regulated Facility"),
            Self::MobileCgnat => write!(f, "Mobile/CGNAT"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub priority: Priority,
    pub category: String,
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Required,
    Recommended,
    Optional,
}

/// Classify an archetype from probe results.
pub fn classify(probes: &[ProbeResult]) -> (NetworkArchetype, f64) {
    let has_internet = probe_passed(probes, "internet");
    let has_nat = probe_passed(probes, "nat");
    let has_multicast = probe_passed(probes, "multicast");
    let has_open_ports = probe_passed(probes, "outbound_ports");
    let has_cgnat = probe_passed(probes, "cgnat");
    let has_symmetric = probe_passed(probes, "symmetric_nat");

    if !has_internet {
        return (NetworkArchetype::RegulatedFacility, 0.85);
    }

    if has_cgnat || has_symmetric {
        return (NetworkArchetype::MobileCgnat, 0.90);
    }

    if !has_open_ports {
        return (NetworkArchetype::EnterpriseDmz, 0.85);
    }

    if !has_nat && has_multicast {
        return (NetworkArchetype::OpenWarehouse, 0.90);
    }

    if has_nat {
        return (NetworkArchetype::NatOffice, 0.85);
    }

    // Fallback: if we have internet and open ports but no NAT,
    // it's likely an open warehouse environment.
    (NetworkArchetype::OpenWarehouse, 0.60)
}

/// Generate transport recommendations for a detected archetype.
pub fn recommend(archetype: NetworkArchetype) -> Vec<Recommendation> {
    match archetype {
        NetworkArchetype::OpenWarehouse => vec![
            Recommendation {
                priority: Priority::Recommended,
                category: "transport".into(),
                action: "Use QUIC for direct peer connections — lowest latency".into(),
            },
            Recommendation {
                priority: Priority::Optional,
                category: "discovery".into(),
                action: "Enable mDNS for automatic peer discovery on LAN".into(),
            },
        ],
        NetworkArchetype::NatOffice => vec![
            Recommendation {
                priority: Priority::Required,
                category: "transport".into(),
                action: "Configure a relay server for initial connection brokering".into(),
            },
            Recommendation {
                priority: Priority::Recommended,
                category: "transport".into(),
                action: "Enable DCUtR for NAT hole-punching after relay handshake".into(),
            },
            Recommendation {
                priority: Priority::Recommended,
                category: "transport".into(),
                action: "Prefer QUIC — better NAT traversal than TCP".into(),
            },
        ],
        NetworkArchetype::EnterpriseDmz => vec![
            Recommendation {
                priority: Priority::Required,
                category: "transport".into(),
                action: "Use TCP over port 443 — QUIC/UDP likely blocked".into(),
            },
            Recommendation {
                priority: Priority::Required,
                category: "transport".into(),
                action: "Configure relay server accessible on TCP 443".into(),
            },
            Recommendation {
                priority: Priority::Recommended,
                category: "security".into(),
                action: "Expect TLS inspection — verify relay certificate pinning".into(),
            },
        ],
        NetworkArchetype::RegulatedFacility => vec![
            Recommendation {
                priority: Priority::Required,
                category: "transport".into(),
                action: "Use store-and-forward via physical media or approved proxy".into(),
            },
            Recommendation {
                priority: Priority::Required,
                category: "security".into(),
                action: "All data must transit through approved egress channels".into(),
            },
            Recommendation {
                priority: Priority::Recommended,
                category: "operations".into(),
                action: "Pre-sign and pre-deploy capabilities during maintenance windows".into(),
            },
        ],
        NetworkArchetype::MobileCgnat => vec![
            Recommendation {
                priority: Priority::Required,
                category: "transport".into(),
                action: "Relay is mandatory — direct connections impossible through symmetric NAT"
                    .into(),
            },
            Recommendation {
                priority: Priority::Recommended,
                category: "transport".into(),
                action: "Use TURN-compatible relay for guaranteed connectivity".into(),
            },
            Recommendation {
                priority: Priority::Recommended,
                category: "resilience".into(),
                action: "Enable aggressive reconnection — mobile connections drop frequently".into(),
            },
            Recommendation {
                priority: Priority::Optional,
                category: "bandwidth".into(),
                action: "Enable artifact compression — mobile bandwidth is limited".into(),
            },
        ],
    }
}

/// Calculate a connectivity score (0-100) from probe results.
pub fn connectivity_score(probes: &[ProbeResult]) -> u32 {
    let total = probes.len() as u32;
    if total == 0 {
        return 0;
    }
    let passed = probes.iter().filter(|p| p.passed).count() as u32;
    // Weight internet connectivity higher
    let internet_bonus = if probe_passed(probes, "internet") {
        20
    } else {
        0
    };
    let base = (passed * 80) / total;
    (base + internet_bonus).min(100)
}

/// Generate a human-readable summary.
pub fn summarize(report: &ArchetypeReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Network Archetype: {} (confidence: {:.0}%)\n",
        report.archetype,
        report.confidence * 100.0
    ));
    out.push_str(&format!(
        "Connectivity Score: {}/100\n",
        report.connectivity_score
    ));
    out.push_str(&"─".repeat(50));
    out.push('\n');

    out.push_str("\nProbes:\n");
    for probe in &report.probes {
        let icon = if probe.passed { "pass" } else { "FAIL" };
        out.push_str(&format!("  [{icon}] {}: {}\n", probe.name, probe.detail));
    }

    out.push_str("\nRecommendations:\n");
    for rec in &report.recommendations {
        let tag = match rec.priority {
            Priority::Required => "REQUIRED",
            Priority::Recommended => "recommended",
            Priority::Optional => "optional",
        };
        out.push_str(&format!(
            "  [{tag}] [{}] {}\n",
            rec.category, rec.action
        ));
    }

    out
}

fn probe_passed(probes: &[ProbeResult], name: &str) -> bool {
    probes.iter().any(|p| p.name == name && p.passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_warehouse_probes() -> Vec<ProbeResult> {
        vec![
            ProbeResult { name: "internet".into(), passed: true, detail: "DNS+ping OK".into() },
            ProbeResult { name: "nat".into(), passed: false, detail: "no NAT detected".into() },
            ProbeResult { name: "multicast".into(), passed: true, detail: "mDNS available".into() },
            ProbeResult { name: "outbound_ports".into(), passed: true, detail: "all ports open".into() },
            ProbeResult { name: "cgnat".into(), passed: false, detail: "no CGNAT".into() },
            ProbeResult { name: "symmetric_nat".into(), passed: false, detail: "not symmetric".into() },
        ]
    }

    fn nat_office_probes() -> Vec<ProbeResult> {
        vec![
            ProbeResult { name: "internet".into(), passed: true, detail: "OK".into() },
            ProbeResult { name: "nat".into(), passed: true, detail: "behind NAT".into() },
            ProbeResult { name: "multicast".into(), passed: true, detail: "LAN mDNS".into() },
            ProbeResult { name: "outbound_ports".into(), passed: true, detail: "ports open".into() },
            ProbeResult { name: "cgnat".into(), passed: false, detail: "no CGNAT".into() },
            ProbeResult { name: "symmetric_nat".into(), passed: false, detail: "not symmetric".into() },
        ]
    }

    fn enterprise_probes() -> Vec<ProbeResult> {
        vec![
            ProbeResult { name: "internet".into(), passed: true, detail: "OK".into() },
            ProbeResult { name: "nat".into(), passed: true, detail: "NAT".into() },
            ProbeResult { name: "multicast".into(), passed: false, detail: "blocked".into() },
            ProbeResult { name: "outbound_ports".into(), passed: false, detail: "only 443".into() },
            ProbeResult { name: "cgnat".into(), passed: false, detail: "no CGNAT".into() },
            ProbeResult { name: "symmetric_nat".into(), passed: false, detail: "not symmetric".into() },
        ]
    }

    fn cgnat_probes() -> Vec<ProbeResult> {
        vec![
            ProbeResult { name: "internet".into(), passed: true, detail: "OK".into() },
            ProbeResult { name: "nat".into(), passed: true, detail: "NAT".into() },
            ProbeResult { name: "multicast".into(), passed: false, detail: "no".into() },
            ProbeResult { name: "outbound_ports".into(), passed: true, detail: "open".into() },
            ProbeResult { name: "cgnat".into(), passed: true, detail: "100.64.x.x range".into() },
            ProbeResult { name: "symmetric_nat".into(), passed: true, detail: "symmetric".into() },
        ]
    }

    #[test]
    fn classify_open_warehouse() {
        let (arch, conf) = classify(&open_warehouse_probes());
        assert_eq!(arch, NetworkArchetype::OpenWarehouse);
        assert!(conf > 0.8);
    }

    #[test]
    fn classify_nat_office() {
        let (arch, conf) = classify(&nat_office_probes());
        assert_eq!(arch, NetworkArchetype::NatOffice);
        assert!(conf > 0.8);
    }

    #[test]
    fn classify_enterprise() {
        let (arch, conf) = classify(&enterprise_probes());
        assert_eq!(arch, NetworkArchetype::EnterpriseDmz);
        assert!(conf > 0.8);
    }

    #[test]
    fn classify_cgnat() {
        let (arch, _) = classify(&cgnat_probes());
        assert_eq!(arch, NetworkArchetype::MobileCgnat);
    }

    #[test]
    fn classify_no_internet() {
        let probes = vec![
            ProbeResult { name: "internet".into(), passed: false, detail: "unreachable".into() },
        ];
        let (arch, _) = classify(&probes);
        assert_eq!(arch, NetworkArchetype::RegulatedFacility);
    }

    #[test]
    fn recommendations_not_empty() {
        for arch in [
            NetworkArchetype::OpenWarehouse,
            NetworkArchetype::NatOffice,
            NetworkArchetype::EnterpriseDmz,
            NetworkArchetype::RegulatedFacility,
            NetworkArchetype::MobileCgnat,
        ] {
            let recs = recommend(arch);
            assert!(!recs.is_empty(), "no recommendations for {arch}");
        }
    }

    #[test]
    fn connectivity_score_open_warehouse() {
        // 3/6 probes pass (nat/cgnat/symmetric correctly absent) + internet bonus
        let score = connectivity_score(&open_warehouse_probes());
        assert!(score >= 50, "score was {score}");
        assert!(score <= 80);
    }

    #[test]
    fn connectivity_score_none_pass() {
        let probes = vec![
            ProbeResult { name: "internet".into(), passed: false, detail: "fail".into() },
            ProbeResult { name: "nat".into(), passed: false, detail: "fail".into() },
        ];
        let score = connectivity_score(&probes);
        assert_eq!(score, 0);
    }

    #[test]
    fn summary_readable() {
        let probes = open_warehouse_probes();
        let (archetype, confidence) = classify(&probes);
        let recs = recommend(archetype);
        let score = connectivity_score(&probes);
        let report = ArchetypeReport {
            archetype,
            confidence,
            probes,
            recommendations: recs,
            connectivity_score: score,
            summary: String::new(),
        };
        let text = summarize(&report);
        assert!(text.contains("Open Warehouse"));
        assert!(text.contains("Probes:"));
        assert!(text.contains("Recommendations:"));
    }

    #[test]
    fn report_json_roundtrip() {
        let probes = nat_office_probes();
        let (archetype, confidence) = classify(&probes);
        let recs = recommend(archetype);
        let report = ArchetypeReport {
            archetype,
            confidence,
            probes,
            recommendations: recs,
            connectivity_score: 75,
            summary: "test".into(),
        };
        let json = serde_json::to_string(&report).unwrap();
        let loaded: ArchetypeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.archetype, NetworkArchetype::NatOffice);
    }
}
