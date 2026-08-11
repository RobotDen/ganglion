//! WASM component entry point (wasm32 builds only).
//!
//! Runs the archetype classification from *inside* the sandbox using the
//! structured `network-probe` primitives (no raw sockets) plus a NAT/CGNAT
//! heuristic derived from `diagnostics-collect::network-state` interface
//! addresses (RFC 1918 → NAT'd, 100.64/10 → CGNAT). Multicast is not
//! probeable through the WIT surface, so that probe is reported as not
//! measurable — the classifier treats it as absent. Args: `[--json]`.

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::{diagnostics_collect, network_probe};

use crate::{ArchetypeReport, ProbeResult, classify, connectivity_score, recommend, summarize};

struct Component;

/// RFC 1918 / CGNAT address detection over the stringified network-state.
fn address_flags(network_state: &str) -> (bool, bool) {
    let mut private = false;
    let mut cgnat = false;
    // Scan for dotted-quad substrings; the broker's JSON stringification keeps
    // addresses verbatim, so plain text matching is reliable.
    for token in network_state.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        if token.starts_with("10.") && token.matches('.').count() == 3 {
            private = true;
        } else if token.starts_with("192.168.") {
            private = true;
        } else if let Some(rest) = token.strip_prefix("172.") {
            if let Some((second, _)) = rest.split_once('.') {
                if let Ok(n) = second.parse::<u8>() {
                    if (16..=31).contains(&n) {
                        private = true;
                    }
                }
            }
        } else if let Some(rest) = token.strip_prefix("100.") {
            if let Some((second, _)) = rest.split_once('.') {
                if let Ok(n) = second.parse::<u8>() {
                    if (64..=127).contains(&n) {
                        cgnat = true;
                    }
                }
            }
        }
    }
    (private, cgnat)
}

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let mut probes = Vec::new();

        // Internet reachability: ICMP to a well-known anycast address.
        let internet = network_probe::ping("1.1.1.1", 2)
            .map(|p| p.reachable)
            .unwrap_or(false);
        probes.push(ProbeResult {
            name: "internet".into(),
            passed: internet,
            detail: if internet {
                "ping to 1.1.1.1 succeeded".into()
            } else {
                "ping to 1.1.1.1 failed — no direct internet or ICMP blocked".into()
            },
        });

        // Non-443 outbound: TCP 53 to a public resolver.
        let open_ports = network_probe::port_check("8.8.8.8", 53, 3)
            .map(|p| p.open)
            .unwrap_or(false);
        probes.push(ProbeResult {
            name: "outbound_ports".into(),
            passed: open_ports,
            detail: if open_ports {
                "outbound TCP 53 open — egress is not port-restricted".into()
            } else {
                "outbound TCP 53 blocked — restrictive egress firewall".into()
            },
        });

        // DNS resolution.
        let dns = network_probe::dns_lookup("cloudflare.com", "A")
            .map(|d| !d.answers.is_empty())
            .unwrap_or(false);
        probes.push(ProbeResult {
            name: "dns".into(),
            passed: dns,
            detail: if dns {
                "DNS resolution works".into()
            } else {
                "DNS resolution failed".into()
            },
        });

        // NAT / CGNAT heuristics from interface addressing.
        let state_text = diagnostics_collect::network_state()
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|e| e.value)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let (private_addr, cgnat_addr) = address_flags(&state_text);
        probes.push(ProbeResult {
            name: "nat".into(),
            passed: private_addr,
            detail: if private_addr {
                "RFC 1918 interface address — behind NAT".into()
            } else {
                "no private interface address detected".into()
            },
        });
        probes.push(ProbeResult {
            name: "cgnat".into(),
            passed: cgnat_addr,
            detail: if cgnat_addr {
                "100.64/10 interface address — carrier-grade NAT".into()
            } else {
                "no CGNAT address range detected".into()
            },
        });

        // Multicast is not probeable through the WIT surface.
        probes.push(ProbeResult {
            name: "multicast".into(),
            passed: false,
            detail: "multicast probe not available in the sandbox — treated as absent".into(),
        });

        let (archetype, confidence) = classify(&probes);
        let recommendations = recommend(archetype);
        let score = connectivity_score(&probes);
        let mut report = ArchetypeReport {
            archetype,
            confidence,
            probes,
            recommendations,
            connectivity_score: score,
            summary: String::new(),
        };
        report.summary = summarize(&report);

        if args.iter().any(|a| a == "--json") {
            serde_json::to_vec(&report).map_err(|e| e.to_string())
        } else {
            Ok(report.summary.clone().into_bytes())
        }
    }
}

export!(Component);
