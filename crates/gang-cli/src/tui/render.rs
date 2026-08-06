//! Ratatui rendering for the dashboard.
//!
//! `render` is a pure function of `(&DashboardState, &Theme, phase, now)` onto a
//! [`ratatui::Frame`], so every layout — populated, paused, the inspect and
//! help overlays, the first-run panel, the too-small fallback, and the
//! `NO_COLOR` degrade — is exercisable headless via [`render_to_lines`] and
//! `ratatui::backend::TestBackend`.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::state::{DashboardState, PeerStatus, View};
use super::theme::Theme;
use crate::commands::{format_bytes, format_duration};

/// Below this width or height the four-pane grid cannot render legibly and we
/// stack into a single column; below the hard minimum we show a hint instead.
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 16;
const STACK_WIDTH: u16 = 96;

/// Draw the whole dashboard for one frame.
pub fn render(
    f: &mut Frame,
    state: &DashboardState,
    theme: &Theme,
    phase: usize,
    now: DateTime<Utc>,
) {
    let area = f.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(f, area, theme);
        return;
    }

    // Title (3) · body (min) · footer (1). The filter editor borrows the footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);

    render_title(f, chunks[0], state, theme, phase, now);
    render_footer(f, chunks[2], state, theme);

    let body = chunks[1];
    match state.view {
        View::AuditFull => render_audit_fullscreen(f, body, state, theme),
        _ => {
            if state.roster_empty && state.peers.is_empty() {
                render_first_run(f, body, theme);
            } else {
                render_grid(f, body, state, theme, now);
            }
        }
    }

    // Overlays float above the body.
    match state.view {
        View::Help => render_help(f, area, theme),
        View::Inspect => render_inspect(f, area, state, theme, now),
        _ => {}
    }
}

fn block<'a>(title: &'a str, theme: &'a Theme, focused: bool) -> Block<'a> {
    let title_style = if focused {
        theme.accent_bright()
    } else {
        theme.accent()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_set(theme.border_set())
        .border_style(if focused {
            theme.accent_bright()
        } else {
            theme.dim()
        })
        .title(Span::styled(format!(" {title} "), title_style))
}

fn render_title(
    f: &mut Frame,
    area: Rect,
    state: &DashboardState,
    theme: &Theme,
    phase: usize,
    now: DateTime<Utc>,
) {
    let relay = state
        .relay_addr
        .as_deref()
        .map(short_relay)
        .unwrap_or_else(|| "not configured".to_string());
    let live = state.live_count(now);
    let total = state.peers.len();

    // The bordered frame, then two aligned paragraphs inside it so the status
    // chips (PAUSED / stale / pulse) stay right-aligned and never get truncated
    // by a long relay address on the left.
    let frame = Block::default()
        .borders(Borders::ALL)
        .border_set(theme.border_set())
        .border_style(theme.accent())
        .title(Span::styled(
            format!(" gang tui {} fleet dashboard ", theme.dash()),
            theme.accent(),
        ));
    let inner = frame.inner(area);
    f.render_widget(frame, area);

    let left = Line::from(vec![
        Span::styled("relay ", theme.dim()),
        Span::styled(relay, theme.text()),
        Span::styled("   peers ", theme.dim()),
        Span::styled(format!("{live}/{total} live"), theme.text()),
        Span::styled("   up ", theme.dim()),
        Span::styled(format_duration(state.uptime_secs(now)), theme.text()),
    ]);

    let mut right = Vec::new();
    if let Some((robot, dropped)) = &state.gap_notice {
        right.push(Span::styled(
            format!("gap {dropped}\u{2193} {robot}  "),
            theme.warn(),
        ));
    }
    if state.paused {
        right.push(Span::styled(" PAUSED ", theme.deny()));
        right.push(Span::raw("  "));
    }
    if state.feed_stale(now) {
        right.push(Span::styled("[stale feed]", theme.warn()));
    } else {
        right.push(Span::styled(
            format!("{} live", theme.pulse(phase)),
            theme.accent_bright(),
        ));
    }

    f.render_widget(Paragraph::new(left), inner);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        inner,
    );
}

fn render_footer(f: &mut Frame, area: Rect, state: &DashboardState, theme: &Theme) {
    if state.editing_filter {
        let text = state.filter.clone().unwrap_or_default();
        let line = Line::from(vec![
            Span::styled("/filter ", theme.accent()),
            Span::styled(text, theme.text()),
            Span::styled("\u{2588}", theme.accent_bright()),
            Span::styled(
                if theme.mono {
                    "   (Enter apply | Esc cancel)"
                } else {
                    "   (Enter apply \u{00b7} Esc cancel)"
                },
                theme.dim(),
            ),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let up_down = if theme.mono {
        "up/dn"
    } else {
        "\u{2191}\u{2193}"
    };
    let enter = if theme.mono { "Enter" } else { "\u{23ce}" };
    let mut keys: Vec<(&str, &str)> = vec![
        (up_down, "select"),
        ("j/k", "move"),
        (enter, "inspect"),
        ("p", "pause"),
        ("/", "filter"),
        ("a", "audit"),
        ("?", "help"),
        ("q", "quit"),
    ];
    if state.filter.is_some() {
        keys.push(("c", "clear filter"));
    }

    let sep = if theme.mono { " | " } else { " \u{00b7} " };
    let mut spans = Vec::new();
    for (i, (k, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(sep, theme.dim()));
        }
        spans.push(Span::styled(*k, theme.accent_bright()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*label, theme.dim()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_grid(
    f: &mut Frame,
    area: Rect,
    state: &DashboardState,
    theme: &Theme,
    now: DateTime<Utc>,
) {
    // Wide terminals get a 2x2 grid; narrow ones stack into a single column.
    if area.width < STACK_WIDTH {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(20),
            ])
            .split(area);
        render_peers(f, rows[0], state, theme, now);
        render_tunnels(f, rows[1], state, theme);
        render_decisions(f, rows[2], state, theme);
        render_audit(f, rows[3], state, theme, false);
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(cols[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(cols[1]);

    render_peers(f, left[0], state, theme, now);
    render_decisions(f, left[1], state, theme);
    render_tunnels(f, right[0], state, theme);
    render_audit(f, right[1], state, theme, false);
}

fn status_marker(status: PeerStatus, theme: &Theme) -> (&'static str, Style) {
    match status {
        PeerStatus::Live => theme.live_marker(),
        PeerStatus::Transitional => theme.transitional_marker(),
        PeerStatus::Offline => theme.offline_marker(),
    }
}

fn render_peers(
    f: &mut Frame,
    area: Rect,
    state: &DashboardState,
    theme: &Theme,
    now: DateTime<Utc>,
) {
    let visible = state.visible_peers();
    let header = Row::new(vec!["", "peer", "transport", "rtt"]).style(theme.dim());
    let rows: Vec<Row> = visible
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (dot, dot_style) = status_marker(p.status(now), theme);
            let transport = match (&p.transport, p.via_relay) {
                (Some(t), true) if t == "relay" => "relay".to_string(),
                (Some(t), true) => format!("{t}/relay"),
                (Some(t), false) => t.clone(),
                (None, _) => theme.dash().to_string(),
            };
            let rtt = p
                .rtt_ms
                .map(|r| format!("{r}ms"))
                .unwrap_or_else(|| theme.dash().to_string());
            let selected = i == state.selected;
            let name_style = if p.status(now) == PeerStatus::Offline {
                theme.dim()
            } else {
                theme.text()
            };
            let row = Row::new(vec![
                Cell::from(dot).style(dot_style),
                Cell::from(p.name.clone()).style(name_style),
                Cell::from(transport),
                Cell::from(rtt),
            ]);
            if selected {
                row.style(theme.selection())
            } else {
                row
            }
        })
        .collect();

    let title = format!("Peers ({})", visible.len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(12),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .block(block(&title, theme, true));
    f.render_widget(table, area);
}

fn render_tunnels(f: &mut Frame, area: Rect, state: &DashboardState, theme: &Theme) {
    let header = Row::new(vec![
        Cell::from("peer"),
        Cell::from("path"),
        Cell::from(theme.up_label()),
        Cell::from(theme.down_label()),
    ])
    .style(theme.dim());
    let rows: Vec<Row> = state
        .tunnels
        .iter()
        .map(|t| {
            let path = match (t.via_relay, t.transport.as_str()) {
                (true, "relay") => "relay".to_string(),
                (true, tr) => format!("relay ({tr})"),
                (false, tr) => format!("direct ({tr})"),
            };
            let path_style = if t.via_relay {
                theme.warn()
            } else {
                theme.ok()
            };
            Row::new(vec![
                Cell::from(t.peer.clone()),
                Cell::from(path).style(path_style),
                Cell::from(format_bytes(t.bytes_up)),
                Cell::from(format_bytes(t.bytes_down)),
            ])
        })
        .collect();

    let inner = if rows.is_empty() {
        Table::new(
            vec![Row::new(vec![Cell::from(Span::styled(
                "no active tunnels",
                theme.dim(),
            ))])],
            [Constraint::Percentage(100)],
        )
    } else {
        Table::new(
            rows,
            [
                Constraint::Min(10),
                Constraint::Length(16),
                Constraint::Length(10),
                Constraint::Length(10),
            ],
        )
        .header(header)
    };
    f.render_widget(inner.block(block("Tunnels", theme, false)), area);
}

fn decision_line<'a>(d: &'a super::state::DecisionRow, theme: &Theme) -> Line<'a> {
    let (verdict, vstyle) = if d.allow {
        ("ALLOW", theme.ok())
    } else {
        ("DENY ", theme.deny())
    };
    Line::from(vec![
        Span::styled(d.ts.format("%H:%M:%S").to_string(), theme.dim()),
        Span::raw(" "),
        Span::styled(verdict, vstyle),
        Span::raw(" "),
        Span::styled(d.capability_group.clone(), theme.accent_bright()),
        Span::styled(format!("  by {}", d.operator), theme.dim()),
        Span::styled(format!("  {}", d.reason), theme.text()),
    ])
}

fn render_decisions(f: &mut Frame, area: Rect, state: &DashboardState, theme: &Theme) {
    let visible = state.visible_decisions();
    let capacity = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = if visible.is_empty() {
        vec![Line::from(Span::styled(
            "no policy decisions yet \u{2014} deploy or run to see allow/deny live",
            theme.dim(),
        ))]
    } else {
        visible
            .iter()
            .rev()
            .take(capacity.max(1))
            .rev()
            .map(|d| decision_line(d, theme))
            .collect()
    };
    let title = format!("Policy decisions (live) ({})", visible.len());
    f.render_widget(
        Paragraph::new(lines).block(block(&title, theme, false)),
        area,
    );
}

fn audit_line<'a>(a: &'a super::state::AuditRow, theme: &Theme) -> Line<'a> {
    let rstyle = match a.result.as_str() {
        "success" => theme.ok(),
        "policy_denied" | "failed" | "trapped" | "timeout" => theme.deny(),
        _ => theme.text(),
    };
    Line::from(vec![
        Span::styled(a.ts.format("%H:%M:%S").to_string(), theme.dim()),
        Span::raw(" "),
        Span::styled(a.action.clone(), theme.text()),
        Span::styled(format!("  by {}", a.operator), theme.dim()),
        Span::raw("  "),
        Span::styled(a.result.clone(), rstyle),
        Span::styled(format!("  {}ms", a.duration_ms), theme.dim()),
    ])
}

fn render_audit(f: &mut Frame, area: Rect, state: &DashboardState, theme: &Theme, full: bool) {
    let visible = state.visible_audit();
    let capacity = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = if visible.is_empty() {
        vec![Line::from(Span::styled(
            "no audit records yet",
            theme.dim(),
        ))]
    } else {
        visible
            .iter()
            .rev()
            .take(capacity.max(1))
            .rev()
            .map(|a| audit_line(a, theme))
            .collect()
    };
    let title = if full {
        format!("Audit tail \u{2014} fullscreen ({})", visible.len())
    } else {
        format!("Audit tail ({})", visible.len())
    };
    f.render_widget(
        Paragraph::new(lines).block(block(&title, theme, full)),
        area,
    );
}

fn render_audit_fullscreen(f: &mut Frame, area: Rect, state: &DashboardState, theme: &Theme) {
    render_audit(f, area, state, theme, true);
}

fn render_first_run(f: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  No robots registered yet.",
            theme.accent_bright(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Stand up a local fleet in one terminal:",
            theme.text(),
        )),
        Line::from(Span::styled("      gang up", theme.accent())),
        Line::from(""),
        Line::from(Span::styled(
            "  …then point this dashboard at it from another:",
            theme.text(),
        )),
        Line::from(Span::styled(
            "      gang --data-dir <dir> tui",
            theme.accent(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Or enrol a real robot with a one-line pairing token:",
            theme.text(),
        )),
        Line::from(Span::styled("      gang pair", theme.accent())),
        Line::from(""),
        Line::from(Span::styled(
            "  The dashboard fills in as soon as a robot connects.",
            theme.dim(),
        )),
    ];
    let p = Paragraph::new(lines).block(block("Welcome to gang tui", theme, true));
    f.render_widget(p, area);
}

fn render_too_small(f: &mut Frame, area: Rect, theme: &Theme) {
    let text = vec![
        Line::from(Span::styled("terminal too small", theme.warn())),
        Line::from(Span::styled(
            format!("need at least {MIN_WIDTH}x{MIN_HEIGHT}"),
            theme.dim(),
        )),
    ];
    let p = Paragraph::new(text).alignment(Alignment::Center);
    f.render_widget(p, area);
}

/// A centered rectangle `pct_x`×`pct_y` percent of `area`.
fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}

fn render_help(f: &mut Frame, area: Rect, theme: &Theme) {
    let popup = centered(70, 80, area);
    f.render_widget(Clear, popup);
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(format!("  {k:<10}"), theme.accent_bright()),
            Span::styled(d.to_string(), theme.text()),
        ])
    };
    let lines = vec![
        Line::from(Span::styled("  Live fleet dashboard", theme.accent())),
        Line::from(""),
        key("\u{2191}/\u{2193} j k", "select a peer"),
        key(
            "\u{23ce} Enter",
            "inspect the selected peer (caps · decisions · audit)",
        ),
        key(
            "p",
            "pause / resume the live feed (freezes for a clean capture)",
        ),
        key("/", "filter by peer name or text; c clears it"),
        key("a", "audit-only fullscreen view"),
        key("?", "toggle this help"),
        key(
            "q / Esc",
            "quit (Ctrl-C also quits and restores the terminal)",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "  Feed is server-push (instant, ADR-024). The pulse shows a live feed;",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  [stale feed] means no events or stats updates arrived recently.",
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  NO_COLOR renders a monochrome / ASCII theme.",
            theme.dim(),
        )),
    ];
    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(block("Help", theme, true));
    f.render_widget(p, popup);
}

fn render_inspect(
    f: &mut Frame,
    area: Rect,
    state: &DashboardState,
    theme: &Theme,
    now: DateTime<Utc>,
) {
    let popup = centered(75, 80, area);
    f.render_widget(Clear, popup);

    let Some(peer) = state.selected_peer() else {
        let p = Paragraph::new(Line::from(Span::styled("no peer selected", theme.dim())))
            .block(block("Inspect", theme, true));
        f.render_widget(p, popup);
        return;
    };

    let (dot, dstyle) = status_marker(peer.status(now), theme);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(dot, dstyle),
            Span::raw(" "),
            Span::styled(peer.name.clone(), theme.accent_bright()),
            Span::styled(format!("   {}", peer.status(now).label()), theme.dim()),
        ]),
        Line::from(vec![
            Span::styled("version  ", theme.dim()),
            Span::styled(
                peer.version.clone().unwrap_or_else(|| "\u{2014}".into()),
                theme.text(),
            ),
            Span::styled("   uptime  ", theme.dim()),
            Span::styled(
                peer.uptime_secs
                    .map(format_duration)
                    .unwrap_or_else(|| "\u{2014}".into()),
                theme.text(),
            ),
            Span::styled("   rtt  ", theme.dim()),
            Span::styled(
                peer.rtt_ms
                    .map(|r| format!("{r}ms"))
                    .unwrap_or_else(|| "\u{2014}".into()),
                theme.text(),
            ),
        ]),
        Line::from(vec![
            Span::styled("capabilities  ", theme.dim()),
            Span::styled(
                if peer.capabilities.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    peer.capabilities.join(", ")
                },
                theme.text(),
            ),
        ]),
    ];
    if let Some(err) = &peer.last_error {
        lines.push(Line::from(vec![
            Span::styled("feed  ", theme.dim()),
            Span::styled(err.clone(), theme.warn()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "recent policy decisions",
        theme.accent(),
    )));

    let name = peer.name.clone();
    let decs: Vec<_> = state
        .decisions
        .iter()
        .filter(|d| d.robot == name)
        .rev()
        .take(6)
        .collect();
    if decs.is_empty() {
        lines.push(Line::from(Span::styled("  (none)", theme.dim())));
    } else {
        for d in decs.into_iter().rev() {
            lines.push(decision_line(d, theme));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("recent audit", theme.accent())));
    let auds: Vec<_> = state
        .audit
        .iter()
        .filter(|a| a.robot == name)
        .rev()
        .take(6)
        .collect();
    if auds.is_empty() {
        lines.push(Line::from(Span::styled("  (none)", theme.dim())));
    } else {
        for a in auds.into_iter().rev() {
            lines.push(audit_line(a, theme));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Enter/Esc to close",
        theme.dim(),
    )));

    let title = format!("Inspect \u{2014} {name}");
    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(block(&title, theme, true));
    f.render_widget(p, popup);
}

fn short_relay(addr: &str) -> String {
    // Trim the /p2p/<id> tail for a compact title.
    match addr.find("/p2p/") {
        Some(i) => addr[..i].to_string(),
        None => addr.to_string(),
    }
}

/// Render one frame to a fixed-size [`ratatui::backend::TestBackend`] and return
/// the buffer as text lines. This is the headless-test + `--frames` entry point:
/// no TTY, no raw mode, fully deterministic given `now`.
pub fn render_to_lines(
    state: &DashboardState,
    theme: &Theme,
    phase: usize,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|f| render(f, state, theme, phase, now))
        .expect("draw");

    let buffer = terminal.backend().buffer().clone();
    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::FeedMsg;
    use chrono::TimeZone;
    use gang_core::events::{AgentEvent, AuditProjection, PolicyOutcome};
    use gang_core::transport::TransportStats;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap()
    }

    fn pid() -> gang_core::identity::PeerId {
        gang_core::identity::Keypair::generate().peer_id()
    }

    fn projection(started: DateTime<Utc>, ended: DateTime<Utc>) -> AuditProjection {
        use gang_core::audit::{AuditRecord, ExitStatus};
        let rec = AuditRecord {
            operator_peer_id: pid(),
            component_name: "diagnostics".into(),
            component_version: "0.1.0".into(),
            component_hash: "abc".into(),
            capabilities_used: vec!["ganglion:diagnostics/collect".into()],
            started_at: started,
            ended_at: ended,
            exit_status: ExitStatus::Success,
            io_stats: vec![],
        };
        (&rec).into()
    }

    fn populated() -> DashboardState {
        let mut s = DashboardState::new(
            now(),
            &["up-robot".into()],
            Some("/ip4/127.0.0.1/tcp/4001/p2p/12D3KooWabc".into()),
        );
        s.apply(
            FeedMsg::Connected {
                robot: "up-robot".into(),
            },
            now(),
        );
        s.apply_event(
            "up-robot",
            &AgentEvent::PresenceSnapshot {
                seq: 1,
                ganglion_version: "2.1.0".into(),
                uptime_secs: 3720,
                archetype: Some("nat-office".into()),
                installed_capabilities: vec!["diagnostics".into()],
            },
            now(),
        );
        s.apply(
            FeedMsg::Stats {
                robot: "up-robot".into(),
                stats: TransportStats {
                    transport: "quic".into(),
                    via_relay: true,
                    bytes_sent: 4096,
                    bytes_received: 20480,
                    last_rtt_ms: Some(37),
                    ..Default::default()
                },
            },
            now(),
        );
        s.apply_event(
            "up-robot",
            &AgentEvent::PolicyDecision {
                seq: 2,
                ts: now(),
                operator_peer: pid(),
                capability_group: "ganglion:diagnostics/collect".into(),
                decision: PolicyOutcome::Allow,
                reason: "declared + trusted".into(),
            },
            now(),
        );
        s.apply_event(
            "up-robot",
            &AgentEvent::PolicyDecision {
                seq: 3,
                ts: now(),
                operator_peer: pid(),
                capability_group: "ganglion:process/spawn".into(),
                decision: PolicyOutcome::Deny,
                reason: "capability not declared".into(),
            },
            now(),
        );
        s.apply_event(
            "up-robot",
            &AgentEvent::AuditAppended {
                seq: 4,
                record: projection(now(), now() + chrono::Duration::milliseconds(412)),
            },
            now(),
        );
        s
    }

    #[test]
    #[ignore = "manual: prints representative frames for docs/review"]
    fn dump_frames() {
        let s = populated();
        let mut paused = populated();
        paused.toggle_pause(now());
        let mut inspect = populated();
        inspect.view = View::Inspect;
        for (label, st, theme) in [
            ("POPULATED (color-mode symbols)", &s, Theme::resolve(false)),
            ("PAUSED", &paused, Theme::resolve(false)),
            ("INSPECT OVERLAY", &inspect, Theme::resolve(false)),
            ("NO_COLOR (monochrome/ASCII)", &s, Theme::resolve(true)),
        ] {
            println!("\n===== {label} =====");
            for line in render_to_lines(st, &theme, 1, now(), 108, 30) {
                println!("{line}");
            }
        }
    }

    #[test]
    fn populated_dashboard_renders_all_panes() {
        let s = populated();
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 100, 30);
        let joined = lines.join("\n");
        assert!(joined.contains("Peers"));
        assert!(joined.contains("Tunnels"));
        assert!(joined.contains("Policy decisions"));
        assert!(joined.contains("Audit tail"));
        assert!(joined.contains("up-robot"));
        assert!(joined.contains("ALLOW"));
        assert!(joined.contains("DENY"));
    }

    #[test]
    fn paused_shows_indicator() {
        let mut s = populated();
        s.toggle_pause(now());
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 100, 30);
        assert!(lines.join("\n").contains("PAUSED"));
    }

    #[test]
    fn no_color_uses_ascii_borders_and_no_escapes() {
        let s = populated();
        let theme = Theme::resolve(true);
        let lines = render_to_lines(&s, &theme, 1, now(), 100, 30);
        let joined = lines.join("\n");
        // ASCII borders, not rounded Unicode.
        assert!(joined.contains('+'));
        assert!(!joined.contains('\u{256d}')); // ╭ rounded corner absent
        // Status dot degrades to ASCII '*'.
        assert!(!joined.contains('\u{25cf}')); // ● absent
        // The entire monochrome frame must be pure ASCII (no box-drawing,
        // arrows, ellipses, or middots) so plain recorders render it cleanly.
        assert!(
            joined.is_ascii(),
            "NO_COLOR frame contained non-ASCII: {joined:?}"
        );
    }

    #[test]
    fn first_run_panel_points_at_gang_up() {
        let s = DashboardState::new(now(), &[], None);
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 100, 30);
        let joined = lines.join("\n");
        assert!(joined.contains("No robots registered"));
        assert!(joined.contains("gang up"));
    }

    #[test]
    fn inspect_overlay_shows_selected_peer_detail() {
        let mut s = populated();
        s.view = View::Inspect;
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 100, 30);
        let joined = lines.join("\n");
        assert!(joined.contains("Inspect"));
        assert!(joined.contains("diagnostics"));
    }

    #[test]
    fn help_overlay_lists_keys() {
        let mut s = populated();
        s.view = View::Help;
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 100, 30);
        assert!(lines.join("\n").contains("Help"));
    }

    #[test]
    fn too_small_shows_hint() {
        let s = populated();
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 30, 10);
        assert!(lines.join("\n").contains("too small"));
    }

    #[test]
    fn narrow_terminal_stacks_without_panicking() {
        let s = populated();
        let theme = Theme::resolve(false);
        let lines = render_to_lines(&s, &theme, 1, now(), 70, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("Peers"));
        assert!(joined.contains("Audit tail"));
    }
}
