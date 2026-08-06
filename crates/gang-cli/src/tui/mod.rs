//! `gang tui` — the live fleet dashboard (issue #2).
//!
//! Architecture (kept deliberately thin so the core is testable headless):
//!
//! * [`state`] holds [`state::DashboardState`] and the **pure event-fold
//!   reducer**. It has no I/O and no clock of its own — the caller passes
//!   `now` — so folding synthetic [`gang_core::events::AgentEvent`]s into UI
//!   rows is unit-testable without a terminal.
//! * [`render`] draws that state onto a [`ratatui::Frame`]; every layout is
//!   exercised via [`render::render_to_lines`] and `TestBackend`.
//! * [`theme`] resolves the teal theme or the `NO_COLOR` monochrome/ASCII
//!   degrade.
//! * This module is the **shell**: it opens one subscription task per robot on
//!   a tokio runtime, funnels [`state::FeedMsg`]s over an mpsc channel, and
//!   runs the crossterm input + render loop. The UI thread never blocks on the
//!   network — a slow or dead robot only delays its own task.
//!
//! Feed delivery follows ADR-024: the subscription is a genuine server-push
//! substream, so events reach the dashboard the instant the robot emits them
//! (no poll cadence). Heartbeats (every 15 s) still drive the title bar's
//! live-pulse / `[stale feed]` indicator, so a stalled or dead feed is visible
//! rather than silently frozen; only the feed latency changed, not liveness.

mod render;
mod state;
mod theme;

use std::io::{Stdout, Write};

use anyhow::Context;
use chrono::Utc;
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use self::state::{DashboardState, FeedMsg, View};
use self::theme::Theme;
use crate::OutputFormat;
use crate::commands::{
    CONTROL_TIMEOUT_SECS, RemoteTarget, ResolvedTarget, establish_remote_connection, prepare_remote,
};

/// The dashboard's redraw tick — drives the heartbeat pulse and staleness
/// readout independent of feed arrivals.
const UI_TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// How often each feed task re-reads its robot's transport stats for the
/// tunnel/RTT panes. The event feed itself is push (no cadence); only the
/// stats side-channel is sampled.
const STATS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Per-cycle fold duration for the headless `--frames N` snapshot. With push
/// delivery there is no poll cycle, so a "cycle" is a fixed slice of wall-clock
/// time the feed is folded for before the frame is rendered.
const SNAPSHOT_CYCLE: std::time::Duration = std::time::Duration::from_secs(1);

/// Default frame size for the headless snapshot (`--frames`).
const SNAPSHOT_W: u16 = 108;
const SNAPSHOT_H: u16 = 34;

/// `gang tui` entry point.
///
/// * `robot_filter` restricts the dashboard to a single registered robot
///   (`--robot`).
/// * `frames` selects headless snapshot mode: fold the feed for that many poll
///   cycles, print the rendered frame as text, and exit (no raw terminal —
///   safe for CI, pipes, and demo capture).
/// * `no_input` runs the live loop but ignores keys (for unattended recording).
/// * `events_transport` selects the feed transport (ADR-024): auto/push/poll.
pub async fn tui(
    robot_filter: Option<&str>,
    frames: Option<usize>,
    no_input: bool,
    events_transport: Option<gang_libp2p::EventsTransport>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    if matches!(format, OutputFormat::Json) {
        anyhow::bail!("`gang tui` is an interactive dashboard and does not support --format json");
    }

    let mode = events_transport.unwrap_or_default();
    let (robots, relay_addr) = build_roster(robot_filter)?;
    let now = Utc::now();
    let names: Vec<String> = robots.iter().map(|(n, _)| n.clone()).collect();
    let mut state = DashboardState::new(now, &names, relay_addr);

    // One bounded channel drains every robot's feed into the render loop.
    let (tx, mut rx) = mpsc::channel::<FeedMsg>(256);
    let mut tasks = Vec::new();
    for (name, target) in robots {
        let tx = tx.clone();
        tasks.push(tokio::spawn(async move {
            feed_task(name, target, mode, tx).await
        }));
    }
    drop(tx); // the loop holds `rx`; tasks hold their clones.

    if let Some(cycles) = frames {
        return snapshot(&mut state, &mut rx, cycles).await;
    }

    let result = run_ui(&mut state, &mut rx, no_input).await;

    for t in tasks {
        t.abort();
    }
    result
}

/// The watchable fleet: named dial targets plus a representative relay address.
type Roster = (Vec<(String, RemoteTarget)>, Option<String>);

/// Resolve the robots to watch and a representative relay address for the title
/// bar. Restricting to `--robot` keeps a focused single-robot dashboard.
fn build_roster(robot_filter: Option<&str>) -> anyhow::Result<Roster> {
    use gang_core::identity::{PeerRegistry, Role, default_registry_path};

    let registry = PeerRegistry::load(&default_registry_path()).unwrap_or_default();
    let mut robots = Vec::new();
    let mut relay_addr = None;

    for (name, entry) in registry.list() {
        if !matches!(entry.role, Role::RobotAgent) {
            continue;
        }
        if let Some(want) = robot_filter
            && name != want
        {
            continue;
        }
        let resolved = ResolvedTarget {
            peer_id: Some(entry.peer_id.clone()),
            libp2p_id: entry.libp2p_id.clone(),
            relay_addr: entry.relay_addrs.first().cloned(),
            name: Some(name.to_string()),
            is_local: false,
        };
        // A robot missing its dialable id or relay can't be watched; skip it
        // rather than aborting the whole dashboard.
        match prepare_remote(&resolved) {
            Ok(target) => {
                if relay_addr.is_none() {
                    relay_addr = Some(target.relay_addr.clone());
                }
                robots.push((name.to_string(), target));
            }
            Err(_) => continue,
        }
    }

    if let Some(want) = robot_filter
        && robots.is_empty()
    {
        anyhow::bail!(
            "no watchable robot named '{want}' (needs a dialable libp2p id and a relay in the \
             registry). See `gang peer list`."
        );
    }

    robots.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((robots, relay_addr))
}

/// Follow one robot's event feed forever, funnelling [`FeedMsg`]s to the loop.
/// The feed transport is chosen by `mode` (ADR-024): push, poll, or auto (push
/// with poll fallback). Reconnects with backoff on transport failure; exits
/// cleanly once the receiver is gone (dashboard quit).
async fn feed_task(
    name: String,
    target: RemoteTarget,
    mode: gang_libp2p::EventsTransport,
    tx: mpsc::Sender<FeedMsg>,
) {
    let timeout = std::time::Duration::from_secs(CONTROL_TIMEOUT_SECS);
    loop {
        let conn = match establish_remote_connection(&target, timeout).await {
            Ok(c) => c,
            Err(e) => {
                if tx
                    .send(FeedMsg::Disconnected {
                        robot: name.clone(),
                        reason: e.to_string(),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        if tx
            .send(FeedMsg::Connected {
                robot: name.clone(),
            })
            .await
            .is_err()
        {
            conn.close().await;
            return;
        }

        // Open the live feed (push or poll per `mode`). A separate ticker
        // samples transport stats for the tunnel/RTT panes.
        let mut stream = match conn
            .transport
            .subscribe_events(&target.gang_id, None, timeout, mode)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(FeedMsg::Disconnected {
                        robot: name.clone(),
                        reason: e.to_string(),
                    })
                    .await;
                conn.close().await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        // Report the active transport for the title-bar indicator; re-report if
        // `auto` falls back push→poll mid-session (checked on each stats tick).
        let mut active = stream.active_transport();
        let _ = tx.send(FeedMsg::Transport { transport: active }).await;

        let mut stats_tick = tokio::time::interval(STATS_INTERVAL);
        stats_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                next = stream.next() => match next {
                    Some(event) => {
                        if tx
                            .send(FeedMsg::Event {
                                robot: name.clone(),
                                event,
                            })
                            .await
                            .is_err()
                        {
                            drop(stream);
                            conn.close().await;
                            return;
                        }
                    }
                    None => {
                        // The robot closed the feed; reconnect after a pause.
                        let _ = tx
                            .send(FeedMsg::Disconnected {
                                robot: name.clone(),
                                reason: "event feed closed".into(),
                            })
                            .await;
                        break;
                    }
                },
                _ = stats_tick.tick() => {
                    // Surface an auto push→poll fallback that happened mid-session.
                    let now_active = stream.active_transport();
                    if now_active != active {
                        active = now_active;
                        let _ = tx
                            .send(FeedMsg::Transport { transport: active })
                            .await;
                    }
                    if let Some(stats) = gang_core::transport::TransportAdapter::transport_stats(
                        conn.transport.as_ref(),
                        &target.gang_id,
                    )
                    .await
                        && tx
                            .send(FeedMsg::Stats {
                                robot: name.clone(),
                                stats,
                            })
                            .await
                            .is_err()
                    {
                        drop(stream);
                        conn.close().await;
                        return;
                    }
                }
            }
        }
        drop(stream);
        conn.close().await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Headless snapshot: fold the feed for `cycles` poll intervals, then print the
/// rendered frame as text and exit. No raw terminal — safe in CI and pipes.
async fn snapshot(
    state: &mut DashboardState,
    rx: &mut mpsc::Receiver<FeedMsg>,
    cycles: usize,
) -> anyhow::Result<()> {
    let budget = SNAPSHOT_CYCLE * (cycles.max(1) as u32) + std::time::Duration::from_secs(3);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            maybe = rx.recv() => match maybe {
                Some(msg) => state.ingest(msg, Utc::now()),
                None => break,
            }
        }
    }
    let theme = Theme::resolve(false);
    let lines = render::render_to_lines(state, &theme, 1, Utc::now(), SNAPSHOT_W, SNAPSHOT_H);
    let mut out = std::io::stdout();
    for line in lines {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// A RAII guard that restores the terminal on drop — normal exit, error, or
/// panic (paired with a panic hook) all leave raw mode and the alternate
/// screen cleanly, so the operator's shell is never left garbled.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
        enable_raw_mode().context("enabling raw mode")?;
        let mut out = std::io::stdout();
        execute!(out, EnterAlternateScreen, cursor::Hide).context("entering alternate screen")?;
        let terminal =
            Terminal::new(CrosstermBackend::new(out)).context("initializing terminal backend")?;
        Ok(terminal)
    }

    /// Idempotent restore, callable from Drop and the panic hook.
    fn restore() {
        let _ = disable_raw_mode();
        let mut out = std::io::stdout();
        let _ = execute!(out, LeaveAlternateScreen, cursor::Show);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}

/// Run the interactive dashboard: crossterm input + feed + redraw tick, all in
/// one tokio select loop so the UI never blocks on the network.
async fn run_ui(
    state: &mut DashboardState,
    rx: &mut mpsc::Receiver<FeedMsg>,
    no_input: bool,
) -> anyhow::Result<()> {
    // Restore the terminal even if a later panic escapes the loop.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        prev_hook(info);
    }));

    let _guard = TerminalGuard;
    let mut terminal = TerminalGuard::enter()?;
    let theme = Theme::resolve(false);

    let mut reader = EventStream::new();
    let mut tick = tokio::time::interval(UI_TICK);
    let mut phase: usize = 0;

    loop {
        let now = Utc::now();
        terminal
            .draw(|f| render::render(f, state, &theme, phase, now))
            .context("drawing frame")?;

        tokio::select! {
            maybe = rx.recv() => {
                // `None` means all feeds ended; keep the UI up until the user
                // quits. On a message, drain any other ready messages this
                // wake-up to keep the fold cheap and the frame current.
                if let Some(msg) = maybe {
                    state.ingest(msg, Utc::now());
                    while let Ok(msg) = rx.try_recv() {
                        state.ingest(msg, Utc::now());
                    }
                }
            }
            maybe_ev = reader.next(), if !no_input => {
                if let Some(Ok(ev)) = maybe_ev
                    && handle_event(state, ev)
                {
                    break;
                }
            }
            _ = tick.tick() => {
                phase = phase.wrapping_add(1);
            }
        }
    }
    Ok(())
}

/// Handle one input event. Returns `true` when the dashboard should quit.
fn handle_event(state: &mut DashboardState, ev: Event) -> bool {
    let key = match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => k,
        _ => return false,
    };
    let now = Utc::now();

    // Ctrl-C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    // Filter editing captures most keys.
    if state.editing_filter {
        match key.code {
            KeyCode::Enter => {
                state.editing_filter = false;
                if state.filter.as_deref() == Some("") {
                    state.filter = None;
                }
            }
            KeyCode::Esc => {
                state.editing_filter = false;
                state.filter = None;
            }
            KeyCode::Backspace => {
                if let Some(f) = state.filter.as_mut() {
                    f.pop();
                }
            }
            KeyCode::Char(c) => {
                state.filter.get_or_insert_with(String::new).push(c);
                state.selected = 0;
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Esc => {
            if state.view == View::Dashboard {
                return true;
            }
            state.view = View::Dashboard;
        }
        KeyCode::Up | KeyCode::Char('k') => state.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => state.move_selection(1),
        KeyCode::Enter => {
            state.view = if state.view == View::Inspect {
                View::Dashboard
            } else {
                View::Inspect
            };
        }
        KeyCode::Char('p') => state.toggle_pause(now),
        KeyCode::Char('/') => {
            state.editing_filter = true;
            state.filter.get_or_insert_with(String::new);
        }
        KeyCode::Char('a') => {
            state.view = if state.view == View::AuditFull {
                View::Dashboard
            } else {
                View::AuditFull
            };
        }
        KeyCode::Char('?') => {
            state.view = if state.view == View::Help {
                View::Dashboard
            } else {
                View::Help
            };
        }
        KeyCode::Char('c') => {
            state.filter = None;
            state.selected = 0;
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::DashboardState;

    fn now() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap()
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::empty()))
    }

    #[test]
    fn q_quits() {
        let mut s = DashboardState::new(now(), &["r".into()], None);
        assert!(handle_event(&mut s, key(KeyCode::Char('q'))));
    }

    #[test]
    fn ctrl_c_quits() {
        let mut s = DashboardState::new(now(), &["r".into()], None);
        let ev = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        assert!(handle_event(&mut s, ev));
    }

    #[test]
    fn p_toggles_pause() {
        let mut s = DashboardState::new(now(), &["r".into()], None);
        assert!(!handle_event(&mut s, key(KeyCode::Char('p'))));
        assert!(s.paused);
        handle_event(&mut s, key(KeyCode::Char('p')));
        assert!(!s.paused);
    }

    #[test]
    fn slash_enters_filter_and_enter_applies() {
        let mut s = DashboardState::new(now(), &["alpha".into(), "beta".into()], None);
        handle_event(&mut s, key(KeyCode::Char('/')));
        assert!(s.editing_filter);
        handle_event(&mut s, key(KeyCode::Char('a')));
        handle_event(&mut s, key(KeyCode::Char('l')));
        handle_event(&mut s, key(KeyCode::Enter));
        assert!(!s.editing_filter);
        assert_eq!(s.filter.as_deref(), Some("al"));
        assert_eq!(s.visible_peers().len(), 1);
    }

    #[test]
    fn esc_closes_overlay_then_quits() {
        let mut s = DashboardState::new(now(), &["r".into()], None);
        s.view = View::Help;
        assert!(!handle_event(&mut s, key(KeyCode::Esc)));
        assert_eq!(s.view, View::Dashboard);
        assert!(handle_event(&mut s, key(KeyCode::Esc)));
    }

    #[test]
    fn enter_toggles_inspect() {
        let mut s = DashboardState::new(now(), &["r".into()], None);
        handle_event(&mut s, key(KeyCode::Enter));
        assert_eq!(s.view, View::Inspect);
        handle_event(&mut s, key(KeyCode::Enter));
        assert_eq!(s.view, View::Dashboard);
    }

    #[test]
    fn a_toggles_audit_fullscreen() {
        let mut s = DashboardState::new(now(), &["r".into()], None);
        handle_event(&mut s, key(KeyCode::Char('a')));
        assert_eq!(s.view, View::AuditFull);
    }
}
