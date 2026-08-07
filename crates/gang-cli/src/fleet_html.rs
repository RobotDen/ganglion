//! A minimal, self-contained fleet-status page rendered from local state.
//!
//! This is deliberately *not* a Formant-style live dashboard — it is a single
//! static HTML file produced from what already exists on the operator's disk
//! (the peer registry, the capability registry, and the recent audit log), so
//! an engineer can glance at fleet shape or hand a snapshot to a colleague
//! without standing up any backend. `gang tui` remains the live view.
//!
//! The renderer is a pure function of its input data, so it is unit-tested for
//! structure and HTML-escaping without touching the filesystem or network.

use serde::Serialize;

/// One robot/peer row.
#[derive(Debug, Clone, Serialize)]
pub struct PeerRow {
    /// Registered name.
    pub name: String,
    /// Gang peer id.
    pub peer_id: String,
    /// Role label (e.g. "robot-agent", "relay").
    pub role: String,
    /// Known relay multiaddrs, joined for display.
    pub relays: String,
    /// Dialable libp2p id, if known.
    pub libp2p_id: String,
}

/// One recent audit row.
#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    /// Component name.
    pub component: String,
    /// Component version.
    pub version: String,
    /// Invoking operator (abbreviated).
    pub operator: String,
    /// Terminal status label.
    pub status: String,
    /// Start time (RFC3339).
    pub started_at: String,
    /// Capabilities used, joined.
    pub capabilities: String,
}

/// Everything the fleet-status page renders.
#[derive(Debug, Clone, Serialize)]
pub struct FleetStatus {
    /// `gang` version string.
    pub version: String,
    /// Operator identity (peer id or a "not generated" note).
    pub identity: String,
    /// Default relay, if configured.
    pub default_relay: Option<String>,
    /// Number of registered capabilities.
    pub registry_count: usize,
    /// Peer rows.
    pub peers: Vec<PeerRow>,
    /// Recent audit rows (already truncated + ordered newest-first by caller).
    pub recent_audit: Vec<AuditRow>,
    /// Generation timestamp (RFC3339), stamped by the caller.
    pub generated_at: String,
}

/// Escape the five HTML-significant characters for safe interpolation.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render the fleet-status page as a single self-contained HTML document.
pub fn render(status: &FleetStatus) -> String {
    let mut peer_rows = String::new();
    if status.peers.is_empty() {
        peer_rows.push_str("<tr><td colspan=\"4\" class=\"empty\">No peers registered yet — enroll one with <code>gang pair</code>.</td></tr>");
    } else {
        for p in &status.peers {
            let libp2p = if p.libp2p_id.is_empty() {
                "<span class=\"muted\">—</span>".to_string()
            } else {
                format!("<code>{}</code>", esc(&p.libp2p_id))
            };
            let relays = if p.relays.is_empty() {
                "<span class=\"muted\">—</span>".to_string()
            } else {
                format!("<code>{}</code>", esc(&p.relays))
            };
            peer_rows.push_str(&format!(
                "<tr><td><strong>{}</strong><br><code class=\"muted\">{}</code></td>\
                 <td><span class=\"pill\">{}</span></td><td>{}</td><td>{}</td></tr>",
                esc(&p.name),
                esc(&p.peer_id),
                esc(&p.role),
                relays,
                libp2p,
            ));
        }
    }

    let mut audit_rows = String::new();
    if status.recent_audit.is_empty() {
        audit_rows
            .push_str("<tr><td colspan=\"5\" class=\"empty\">No audit records yet.</td></tr>");
    } else {
        for a in &status.recent_audit {
            let status_class = if a.status == "success" { "ok" } else { "bad" };
            audit_rows.push_str(&format!(
                "<tr><td class=\"mono\">{}</td><td>{} <span class=\"muted\">{}</span></td>\
                 <td><code class=\"muted\">{}</code></td><td><span class=\"pill {}\">{}</span></td>\
                 <td><code class=\"muted\">{}</code></td></tr>",
                esc(&a.started_at),
                esc(&a.component),
                esc(&a.version),
                esc(&a.operator),
                status_class,
                esc(&a.status),
                esc(&a.capabilities),
            ));
        }
    }

    let relay = status
        .default_relay
        .as_deref()
        .map(esc)
        .unwrap_or_else(|| "<span class=\"muted\">not set</span>".to_string());

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Ganglion fleet status</title>
<style>
  :root {{ color-scheme: dark; --bg:#0e1414; --panel:#151d1d; --line:#243030;
           --fg:#e6f0ee; --muted:#7d918d; --teal:#2dd4bf; --ok:#2dd4bf; --bad:#f87171; }}
  * {{ box-sizing: border-box; }}
  body {{ margin:0; background:var(--bg); color:var(--fg);
          font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace; padding:2rem; }}
  h1 {{ font-size:1.3rem; margin:0 0 .25rem; }}
  h2 {{ font-size:1rem; margin:2rem 0 .75rem; color:var(--teal); font-weight:600; }}
  .sub {{ color:var(--muted); margin:0 0 1.5rem; }}
  .grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:1rem; }}
  .card {{ background:var(--panel); border:1px solid var(--line); border-radius:10px; padding:1rem; }}
  .card .k {{ color:var(--muted); font-size:.8rem; }}
  .card .v {{ font-size:1.1rem; margin-top:.25rem; word-break:break-all; }}
  table {{ width:100%; border-collapse:collapse; background:var(--panel);
           border:1px solid var(--line); border-radius:10px; overflow:hidden; }}
  th,td {{ text-align:left; padding:.6rem .8rem; border-bottom:1px solid var(--line); vertical-align:top; }}
  th {{ color:var(--muted); font-weight:600; font-size:.8rem; text-transform:uppercase; letter-spacing:.04em; }}
  tr:last-child td {{ border-bottom:none; }}
  code {{ font-size:.85em; }}
  .muted {{ color:var(--muted); }}
  .mono {{ white-space:nowrap; }}
  .empty {{ color:var(--muted); text-align:center; padding:1.25rem; }}
  .pill {{ display:inline-block; padding:.1rem .5rem; border-radius:999px;
           background:#1d2b2b; border:1px solid var(--line); font-size:.8rem; }}
  .pill.ok {{ color:var(--ok); border-color:#1f4d47; }}
  .pill.bad {{ color:var(--bad); border-color:#4d1f1f; }}
  footer {{ color:var(--muted); margin-top:2rem; font-size:.8rem; }}
</style>
</head>
<body>
  <h1>Ganglion fleet status</h1>
  <p class="sub">Snapshot from local state · generated {generated}</p>

  <div class="grid">
    <div class="card"><div class="k">gang version</div><div class="v">{version}</div></div>
    <div class="card"><div class="k">operator identity</div><div class="v">{identity}</div></div>
    <div class="card"><div class="k">registered peers</div><div class="v">{peer_count}</div></div>
    <div class="card"><div class="k">capabilities</div><div class="v">{registry_count}</div></div>
    <div class="card"><div class="k">default relay</div><div class="v">{relay}</div></div>
  </div>

  <h2>Peers</h2>
  <table>
    <thead><tr><th>Name / peer id</th><th>Role</th><th>Relays</th><th>libp2p id</th></tr></thead>
    <tbody>{peer_rows}</tbody>
  </table>

  <h2>Recent audit</h2>
  <table>
    <thead><tr><th>Started</th><th>Component</th><th>Operator</th><th>Status</th><th>Capabilities</th></tr></thead>
    <tbody>{audit_rows}</tbody>
  </table>

  <footer>Ganglion is outbound-only, signed, and default-deny. This page is a
  static snapshot — for a live view use <code>gang tui</code>.</footer>
</body>
</html>
"#,
        generated = esc(&status.generated_at),
        version = esc(&status.version),
        identity = esc(&status.identity),
        peer_count = status.peers.len(),
        registry_count = status.registry_count,
        relay = relay,
        peer_rows = peer_rows,
        audit_rows = audit_rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FleetStatus {
        FleetStatus {
            version: "2.1.0".into(),
            identity: "12D3-abc".into(),
            default_relay: Some("/dns4/relay.example/tcp/443".into()),
            registry_count: 3,
            peers: vec![PeerRow {
                name: "up-robot".into(),
                peer_id: "12D3-def".into(),
                role: "robot-agent".into(),
                relays: "/ip4/1.2.3.4/tcp/4001".into(),
                libp2p_id: "12D3KooWabc".into(),
            }],
            recent_audit: vec![AuditRow {
                component: "diagnostics".into(),
                version: "0.1.0".into(),
                operator: "12D3-ebbc".into(),
                status: "success".into(),
                started_at: "2026-08-07T00:00:00Z".into(),
                capabilities: "ganglion:diagnostics/collect".into(),
            }],
            generated_at: "2026-08-07T12:00:00Z".into(),
        }
    }

    #[test]
    fn renders_key_facts() {
        let html = render(&sample());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("Ganglion fleet status"));
        assert!(html.contains("up-robot"));
        assert!(html.contains("robot-agent"));
        assert!(html.contains("diagnostics"));
        assert!(html.contains("/dns4/relay.example/tcp/443"));
        assert!(html.contains("pill ok")); // success styling
    }

    #[test]
    fn escapes_html_in_data() {
        let mut s = sample();
        s.peers[0].name = "evil<script>\"&".into();
        let html = render(&s);
        assert!(!html.contains("<script>"));
        assert!(html.contains("evil&lt;script&gt;&quot;&amp;"));
    }

    #[test]
    fn empty_fleet_has_friendly_placeholders() {
        let mut s = sample();
        s.peers.clear();
        s.recent_audit.clear();
        let html = render(&s);
        assert!(html.contains("No peers registered yet"));
        assert!(html.contains("No audit records yet"));
    }
}
