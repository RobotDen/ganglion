//! `gang doctor` — print exactly what the customer network permits.
//!
//! Where `gang diagnose` classifies the *network archetype*, `gang doctor`
//! answers the field engineer's operational question directly: **which of the
//! outbound paths Ganglion needs actually work on this network, is the relay
//! reachable, and — if not — what is the minimal thing to ask the network /
//! security team to allow?**
//!
//! Ganglion is outbound-only: a robot or operator only ever *dials out*, so the
//! diagnostic is entirely about egress. The command runs a handful of focused
//! probes, prints a PASS/FAIL table, derives a copy-pasteable egress allowlist
//! ("what to tell your IT team"), and exits non-zero when no viable outbound
//! path to a relay exists — so it drops cleanly into a support thread:
//! *"run `gang doctor` and paste the output."*
//!
//! The multiaddr parsing and the report rendering are pure functions with unit
//! tests; the network probes are thin wrappers over std sockets with short
//! timeouts, run off the async executor via `spawn_blocking`.

use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::OutputFormat;

/// Default per-probe timeout. Short enough that a full `gang doctor` run over a
/// hostile network still returns in a few seconds.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Transport family a relay endpoint listens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayTransport {
    /// TCP (circuit relay over TCP — the DMZ-friendly path, usually on 443).
    Tcp,
    /// QUIC over UDP (the fast path when UDP egress is permitted).
    Quic,
}

impl RelayTransport {
    /// Human label used in reports and allowlist lines.
    pub fn label(self) -> &'static str {
        match self {
            RelayTransport::Tcp => "TCP",
            RelayTransport::Quic => "UDP/QUIC",
        }
    }
}

/// A relay endpoint distilled from a multiaddr: enough to run a plain socket
/// reachability probe and to name it in an egress allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayEndpoint {
    /// Host as written in the multiaddr — an IP literal or a DNS name.
    pub host: String,
    /// Port the relay listens on.
    pub port: u16,
    /// Transport family.
    pub transport: RelayTransport,
    /// The relay's peer id, if the multiaddr carried a `/p2p/…` component.
    pub peer_id: Option<String>,
}

impl RelayEndpoint {
    /// `host:port` as used for socket connection and allowlist rendering.
    pub fn host_port(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Parse the host / port / transport out of a relay multiaddr.
///
/// Handles the address shapes Ganglion actually emits, e.g.
/// `/dns4/relay.example/tcp/4001`, `/ip4/1.2.3.4/tcp/443/p2p/12D3…`, and
/// `/ip4/1.2.3.4/udp/443/quic-v1/p2p/12D3…`. A trailing `/p2p-circuit` is
/// ignored (we probe the relay's own transport address, not the circuit).
/// Returns `None` for input that does not carry a host + transport + port.
pub fn parse_relay_multiaddr(addr: &str) -> Option<RelayEndpoint> {
    // Multiaddrs are `/proto/value/proto/value/…`. Split into components,
    // dropping the leading empty string from the leading '/'.
    let parts: Vec<&str> = addr.trim().trim_end_matches('/').split('/').collect();
    let parts: Vec<&str> = parts.into_iter().filter(|s| !s.is_empty()).collect();

    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut transport: Option<RelayTransport> = None;
    let mut peer_id: Option<String> = None;
    let mut saw_quic = false;

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "ip4" | "ip6" | "dns" | "dns4" | "dns6" => {
                host = parts.get(i + 1).map(|s| s.to_string());
                i += 2;
            }
            "tcp" => {
                port = parts.get(i + 1).and_then(|s| s.parse().ok());
                // A later `/quic-v1` upgrades the family; default TCP for now.
                transport.get_or_insert(RelayTransport::Tcp);
                i += 2;
            }
            "udp" => {
                port = parts.get(i + 1).and_then(|s| s.parse().ok());
                transport = Some(RelayTransport::Quic);
                i += 2;
            }
            "quic-v1" | "quic" => {
                saw_quic = true;
                transport = Some(RelayTransport::Quic);
                i += 1;
            }
            "p2p" => {
                peer_id = parts.get(i + 1).map(|s| s.to_string());
                i += 2;
            }
            // `p2p-circuit` and anything else we don't model: skip one token.
            _ => i += 1,
        }
    }

    if saw_quic {
        transport = Some(RelayTransport::Quic);
    }

    match (host, port, transport) {
        (Some(host), Some(port), Some(transport)) => Some(RelayEndpoint {
            host,
            port,
            transport,
            peer_id,
        }),
        _ => None,
    }
}

/// Outcome of a single egress probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressCheck {
    /// Stable machine name (e.g. `"outbound_tcp_443"`).
    pub name: String,
    /// Short human title shown in the table.
    pub title: String,
    /// Whether the path is usable.
    pub ok: bool,
    /// Plain-language detail.
    pub detail: String,
}

/// The full `gang doctor` report — serializable for `--json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    /// Egress probe results.
    pub checks: Vec<EgressCheck>,
    /// The relay we tried to reach, if one was configured/supplied.
    pub relay: Option<RelayEndpoint>,
    /// Whether the relay's transport address was reachable (None if no relay).
    pub relay_reachable: Option<bool>,
    /// Whether an operator/robot identity key is present on this host.
    pub identity_present: bool,
    /// Minimal egress allowlist lines ("what to tell your IT team").
    pub allowlist: Vec<String>,
    /// True when at least one outbound path that can reach a relay works.
    pub viable_path: bool,
}

// --- Probes (blocking; wrapped in spawn_blocking by the caller) ---

/// Resolve a host:port to at least one socket address.
fn resolves(host: &str, port: u16) -> bool {
    (host, port)
        .to_socket_addrs()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// Attempt a TCP connection to the first resolved address of `host:port`.
fn tcp_connect(host: &str, port: u16, timeout: Duration) -> bool {
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
}

/// Confirm UDP egress by sending a real DNS query to a public resolver over
/// UDP and awaiting a response. This is the prerequisite for QUIC, so a pass
/// here means the QUIC relay path is worth trying.
fn udp_egress_works(resolver: &str, timeout: Duration) -> bool {
    let Ok(mut addrs) = resolver.to_socket_addrs() else {
        return false;
    };
    let Some(server) = addrs.next() else {
        return false;
    };
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return false;
    };
    if sock.set_read_timeout(Some(timeout)).is_err() {
        return false;
    }
    // Minimal DNS A-query for "one.one.one.one" with a fixed transaction id.
    let query = build_dns_query(0x6741, "one.one.one.one");
    if sock.send_to(&query, server).is_err() {
        return false;
    }
    let mut buf = [0u8; 512];
    match sock.recv_from(&mut buf) {
        // A valid response echoes the transaction id in the first two bytes
        // and sets the QR (response) bit in byte 2.
        Ok((n, _)) if n >= 4 => buf[0] == 0x67 && buf[1] == 0x41 && (buf[2] & 0x80) != 0,
        _ => false,
    }
}

/// Build a minimal DNS query packet (header + single A/IN question).
fn build_dns_query(id: u16, name: &str) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(32);
    pkt.extend_from_slice(&id.to_be_bytes()); // transaction id
    pkt.extend_from_slice(&0x0100u16.to_be_bytes()); // flags: recursion desired
    pkt.extend_from_slice(&1u16.to_be_bytes()); // qdcount
    pkt.extend_from_slice(&0u16.to_be_bytes()); // ancount
    pkt.extend_from_slice(&0u16.to_be_bytes()); // nscount
    pkt.extend_from_slice(&0u16.to_be_bytes()); // arcount
    for label in name.split('.') {
        pkt.push(label.len() as u8);
        pkt.extend_from_slice(label.as_bytes());
    }
    pkt.push(0); // root label
    pkt.extend_from_slice(&1u16.to_be_bytes()); // qtype A
    pkt.extend_from_slice(&1u16.to_be_bytes()); // qclass IN
    pkt
}

/// A resolver check: DNS resolution working at all.
fn dns_works() -> bool {
    resolves("one.one.one.one", 443) || resolves("cloudflare.com", 443)
}

/// Run every probe and assemble the report. Blocking; call via `spawn_blocking`.
fn build_report(relay_addr: Option<String>, identity_present: bool) -> DoctorReport {
    let mut checks = Vec::new();

    // 1) Outbound TCP 443 — the universally-permitted, DMZ-friendly path.
    let tcp443 =
        tcp_connect("1.1.1.1", 443, PROBE_TIMEOUT) || tcp_connect("8.8.8.8", 443, PROBE_TIMEOUT);
    checks.push(EgressCheck {
        name: "outbound_tcp_443".into(),
        title: "Outbound TCP 443".into(),
        ok: tcp443,
        detail: if tcp443 {
            "HTTPS-port egress works — a relay on TCP 443 is reachable from here.".into()
        } else {
            "No outbound TCP 443 — even HTTPS egress is blocked; escalate to network team.".into()
        },
    });

    // 2) Outbound UDP / QUIC — the fast path.
    let udp = udp_egress_works("1.1.1.1:53", PROBE_TIMEOUT);
    checks.push(EgressCheck {
        name: "outbound_udp_quic".into(),
        title: "Outbound UDP (QUIC)".into(),
        ok: udp,
        detail: if udp {
            "UDP egress works — the QUIC relay path (faster, fewer round-trips) is usable.".into()
        } else {
            "UDP egress blocked — QUIC won't work; Ganglion will fall back to TCP relay.".into()
        },
    });

    // 3) Non-443 outbound TCP — tells us how restrictive the firewall is.
    let high_tcp = tcp_connect("8.8.8.8", 53, PROBE_TIMEOUT);
    checks.push(EgressCheck {
        name: "outbound_tcp_other".into(),
        title: "Outbound TCP (non-443)".into(),
        ok: high_tcp,
        detail: if high_tcp {
            "Non-443 TCP egress works — direct dials and relays on any port are viable.".into()
        } else {
            "Non-443 TCP blocked — enterprise firewall; pin the relay to TCP 443.".into()
        },
    });

    // 4) DNS resolution.
    let dns = dns_works();
    checks.push(EgressCheck {
        name: "dns".into(),
        title: "DNS resolution".into(),
        ok: dns,
        detail: if dns {
            "Name resolution works.".into()
        } else {
            "DNS resolution failed — use an ip4/ip6 relay multiaddr instead of a dns name.".into()
        },
    });

    // 5) Relay reachability (if a relay was configured or passed).
    let relay = relay_addr.as_deref().and_then(parse_relay_multiaddr);
    let relay_reachable = relay.as_ref().map(|ep| match ep.transport {
        // For a QUIC relay we can only cheaply confirm the host resolves and
        // UDP egress works; a TCP relay we probe directly.
        RelayTransport::Tcp => tcp_connect(&ep.host, ep.port, PROBE_TIMEOUT),
        RelayTransport::Quic => udp && resolves(&ep.host, ep.port),
    });
    if let (Some(ep), Some(reachable)) = (relay.as_ref(), relay_reachable) {
        checks.push(EgressCheck {
            name: "relay".into(),
            title: format!("Relay reachability ({})", ep.transport.label()),
            ok: reachable,
            detail: if reachable {
                format!("Relay {} is reachable.", ep.host_port())
            } else {
                format!(
                    "Relay {} is NOT reachable over {} — check the address and the egress \
                     allowlist below.",
                    ep.host_port(),
                    ep.transport.label()
                )
            },
        });
    }

    // 6) Identity presence.
    checks.push(EgressCheck {
        name: "identity".into(),
        title: "Operator/robot identity".into(),
        ok: identity_present,
        detail: if identity_present {
            "Identity key present at ~/.gang/identity.key.".into()
        } else {
            "No identity yet — run `gang init` (operator) or `gang join <token>` (robot).".into()
        },
    });

    let allowlist = derive_allowlist(relay.as_ref());

    // A path is viable if the specific relay is reachable, or — when no relay
    // is configured — if at least HTTPS-port egress works (a relay on 443
    // would be reachable). UDP-only counts too.
    let viable_path = match relay_reachable {
        Some(reachable) => reachable,
        None => tcp443 || udp,
    };

    DoctorReport {
        checks,
        relay,
        relay_reachable,
        identity_present,
        allowlist,
        viable_path,
    }
}

/// Build the minimal egress allowlist to hand to a network/security team.
pub fn derive_allowlist(relay: Option<&RelayEndpoint>) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(
        "Ganglion is outbound-only: NO inbound ports need to be opened on the robot's network."
            .to_string(),
    );
    match relay {
        Some(ep) => {
            let proto = match ep.transport {
                RelayTransport::Tcp => "TCP",
                RelayTransport::Quic => "UDP",
            };
            lines.push(format!(
                "Allow outbound {proto} to {} (the Ganglion relay).",
                ep.host_port()
            ));
            if ep.transport == RelayTransport::Quic {
                lines.push(format!(
                    "Optional TCP fallback: allow outbound TCP to {}:{} as well.",
                    ep.host, ep.port
                ));
            }
        }
        None => {
            lines.push(
                "No relay is configured yet. When you set one, allow outbound TCP 443 to the \
                 relay host (and UDP 443 too if you want the faster QUIC path)."
                    .to_string(),
            );
        }
    }
    lines
}

/// Render the report as a human-readable text block.
pub fn render_text(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str("============================================\n");
    out.push_str("  gang doctor — outbound reachability\n");
    out.push_str("============================================\n\n");

    for c in &report.checks {
        let mark = if c.ok { "PASS" } else { "FAIL" };
        out.push_str(&format!("  [{mark}] {}\n", c.title));
        out.push_str(&format!("         {}\n", c.detail));
    }
    out.push('\n');

    if report.relay_reachable == Some(false) {
        out.push_str(
            "  Relay unreachable. The most common cause is an egress firewall — share the \
             allowlist below with the network team.\n\n",
        );
    }

    out.push_str("What to tell your network / security team:\n");
    for line in &report.allowlist {
        out.push_str(&format!("  • {line}\n"));
    }
    out.push('\n');

    if report.viable_path {
        out.push_str(
            "Verdict: a viable outbound path exists. You should be able to pair/enroll.\n",
        );
    } else {
        out.push_str(
            "Verdict: NO viable outbound path found. Ganglion cannot reach a relay from here \
             until the allowlist above is applied.\n",
        );
    }
    out
}

/// `gang doctor` entry point.
pub async fn doctor(relay: Option<&str>, format: &OutputFormat) -> anyhow::Result<()> {
    // Resolve the relay: explicit --relay wins, else config `default_relay`.
    let relay_addr = relay.map(|s| s.to_string()).or_else(|| {
        crate::commands::OperatorConfig::load()
            .default_relay
            .clone()
    });

    let identity_present = gang_core::identity::default_key_path().exists();

    if matches!(format, OutputFormat::Text) {
        println!("Running outbound reachability probes (this may take a few seconds)...\n");
    }

    let relay_for_probe = relay_addr.clone();
    let report =
        tokio::task::spawn_blocking(move || build_report(relay_for_probe, identity_present))
            .await
            .map_err(|e| anyhow::anyhow!("probe task failed: {e}"))?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Text => {
            print!("{}", render_text(&report));
        }
    }

    // Non-zero exit when there is no viable outbound path, so the command is
    // usable as a gate in scripts and CI.
    if !report.viable_path {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tcp_dns_relay() {
        let ep = parse_relay_multiaddr("/dns4/relay.gang.tafy.dev/tcp/4001").unwrap();
        assert_eq!(ep.host, "relay.gang.tafy.dev");
        assert_eq!(ep.port, 4001);
        assert_eq!(ep.transport, RelayTransport::Tcp);
        assert_eq!(ep.peer_id, None);
    }

    #[test]
    fn parses_tcp_ip4_with_peer() {
        let ep = parse_relay_multiaddr(
            "/ip4/1.2.3.4/tcp/443/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk",
        )
        .unwrap();
        assert_eq!(ep.host, "1.2.3.4");
        assert_eq!(ep.port, 443);
        assert_eq!(ep.transport, RelayTransport::Tcp);
        assert_eq!(
            ep.peer_id.as_deref(),
            Some("12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk")
        );
    }

    #[test]
    fn parses_quic_udp() {
        let ep = parse_relay_multiaddr("/ip4/1.2.3.4/udp/443/quic-v1/p2p/12D3KooW").unwrap();
        assert_eq!(ep.host, "1.2.3.4");
        assert_eq!(ep.port, 443);
        assert_eq!(ep.transport, RelayTransport::Quic);
        assert_eq!(ep.peer_id.as_deref(), Some("12D3KooW"));
    }

    #[test]
    fn ignores_p2p_circuit_suffix() {
        let ep = parse_relay_multiaddr("/ip4/10.0.0.5/tcp/4001/p2p/12D3KooW/p2p-circuit").unwrap();
        assert_eq!(ep.host, "10.0.0.5");
        assert_eq!(ep.port, 4001);
        assert_eq!(ep.transport, RelayTransport::Tcp);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_relay_multiaddr("not-a-multiaddr").is_none());
        assert!(parse_relay_multiaddr("").is_none());
        assert!(parse_relay_multiaddr("/ip4/1.2.3.4").is_none()); // no transport/port
    }

    #[test]
    fn dns_query_is_well_formed() {
        let q = build_dns_query(0x6741, "one.one.one.one");
        assert_eq!(&q[0..2], &[0x67, 0x41]); // id
        assert_eq!(&q[2..4], &[0x01, 0x00]); // flags: RD
        assert_eq!(&q[4..6], &[0x00, 0x01]); // qdcount = 1
        // Ends with root label + qtype(A=1) + qclass(IN=1).
        assert_eq!(&q[q.len() - 5..], &[0x00, 0x00, 0x01, 0x00, 0x01]);
    }

    #[test]
    fn allowlist_names_the_tcp_relay() {
        let ep = parse_relay_multiaddr("/dns4/relay.example/tcp/443").unwrap();
        let lines = derive_allowlist(Some(&ep));
        assert!(lines.iter().any(|l| l.contains("outbound-only")));
        assert!(lines.iter().any(|l| l.contains("TCP to relay.example:443")));
    }

    #[test]
    fn allowlist_quic_offers_tcp_fallback() {
        let ep = parse_relay_multiaddr("/ip4/1.2.3.4/udp/443/quic-v1").unwrap();
        let lines = derive_allowlist(Some(&ep));
        assert!(lines.iter().any(|l| l.contains("UDP to 1.2.3.4:443")));
        assert!(lines.iter().any(|l| l.contains("TCP fallback")));
    }

    #[test]
    fn allowlist_without_relay_gives_generic_guidance() {
        let lines = derive_allowlist(None);
        assert!(lines.iter().any(|l| l.contains("No relay is configured")));
    }

    #[test]
    fn render_text_marks_pass_and_fail_and_verdict() {
        let report = DoctorReport {
            checks: vec![
                EgressCheck {
                    name: "outbound_tcp_443".into(),
                    title: "Outbound TCP 443".into(),
                    ok: true,
                    detail: "ok".into(),
                },
                EgressCheck {
                    name: "outbound_udp_quic".into(),
                    title: "Outbound UDP (QUIC)".into(),
                    ok: false,
                    detail: "blocked".into(),
                },
            ],
            relay: None,
            relay_reachable: None,
            identity_present: false,
            allowlist: derive_allowlist(None),
            viable_path: true,
        };
        let text = render_text(&report);
        assert!(text.contains("[PASS] Outbound TCP 443"));
        assert!(text.contains("[FAIL] Outbound UDP (QUIC)"));
        assert!(text.contains("viable outbound path exists"));
    }
}
