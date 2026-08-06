//! The dashboard state and the pure event-fold reducer.
//!
//! Everything the UI draws lives in [`DashboardState`]. The reducer
//! ([`DashboardState::apply`]) folds one [`FeedMsg`] into that state and is
//! deliberately pure — no I/O, no clock of its own (the caller passes `now`) —
//! so the whole fleet-view logic is testable headless by feeding synthetic
//! events and asserting the resulting rows. The event loop is a thin shell over
//! this reducer plus [`DashboardState::ingest`], which adds pause-buffering.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use gang_core::events::{AgentEvent, ConnectionState, PolicyOutcome};
use gang_core::transport::TransportStats;

/// How many policy decisions / audit rows to retain for the tail panes.
const TAIL_CAP: usize = 500;
/// A peer seen within this window is "live".
const LIVE_WINDOW_SECS: i64 = 4;
/// A peer seen within this window (but not the live one) is "transitional".
const TRANSITIONAL_WINDOW_SECS: i64 = 12;

/// A message from a per-robot subscription task into the render loop. Tagging
/// every payload with `robot` lets one reducer fold a whole fleet.
#[derive(Debug, Clone)]
pub enum FeedMsg {
    /// The circuit to `robot` is up and the feed is open.
    Connected { robot: String },
    /// A decoded agent event from `robot`.
    Event { robot: String, event: AgentEvent },
    /// Fresh per-connection transport counters for `robot`.
    Stats {
        robot: String,
        stats: TransportStats,
    },
    /// The feed to `robot` dropped; `reason` is a short human string.
    Disconnected { robot: String, reason: String },
    /// The active event transport (ADR-024). Reported when a feed opens and
    /// again if `auto` falls back push→poll mid-session, so the title bar can
    /// show which path is live. Dashboard-global (last writer wins across the
    /// fleet), so it carries no per-robot key.
    Transport {
        transport: gang_libp2p::EventsTransport,
    },
}

/// Coarse liveness of a peer, derived from how recently it was heard from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStatus {
    /// Heard from within [`LIVE_WINDOW_SECS`].
    Live,
    /// Heard from recently but past the live window, or connecting.
    Transitional,
    /// Not heard from for a while, or the connection went down.
    Offline,
}

impl PeerStatus {
    /// A stable short label for tests and the audit-fullscreen legend.
    pub fn label(self) -> &'static str {
        match self {
            PeerStatus::Live => "live",
            PeerStatus::Transitional => "transitional",
            PeerStatus::Offline => "offline",
        }
    }
}

/// One row in the Peers pane.
#[derive(Debug, Clone)]
pub struct PeerRow {
    /// Registered name (or a short id when unnamed).
    pub name: String,
    /// The robot's ganglion version, once a snapshot arrived.
    pub version: Option<String>,
    /// Agent uptime in seconds, from the latest snapshot/heartbeat.
    pub uptime_secs: Option<u64>,
    /// Installed capability group names, from the presence snapshot.
    pub capabilities: Vec<String>,
    /// Transport in use for the operator↔robot circuit (tcp/quic/relay).
    pub transport: Option<String>,
    /// Whether the connection is relayed.
    pub via_relay: bool,
    /// Latest measured RTT, if the transport reported one.
    pub rtt_ms: Option<u64>,
    /// When we last heard anything from this robot.
    pub last_seen: Option<DateTime<Utc>>,
    /// Set when a `ConnectionChanged{Down}` was observed; forces Offline.
    pub conn_down: bool,
    /// Whether the feed task reports the circuit currently up.
    pub connected: bool,
    /// The most recent feed error for this peer, shown in the inspect overlay.
    pub last_error: Option<String>,
}

impl PeerRow {
    fn new(name: String) -> Self {
        Self {
            name,
            version: None,
            uptime_secs: None,
            capabilities: Vec::new(),
            transport: None,
            via_relay: false,
            rtt_ms: None,
            last_seen: None,
            conn_down: false,
            connected: false,
            last_error: None,
        }
    }

    /// The peer's liveness at instant `now`.
    pub fn status(&self, now: DateTime<Utc>) -> PeerStatus {
        if self.conn_down || !self.connected {
            return PeerStatus::Offline;
        }
        match self.last_seen {
            None => PeerStatus::Transitional,
            Some(seen) => {
                let age = (now - seen).num_seconds();
                if age <= LIVE_WINDOW_SECS {
                    PeerStatus::Live
                } else if age <= TRANSITIONAL_WINDOW_SECS {
                    PeerStatus::Transitional
                } else {
                    PeerStatus::Offline
                }
            }
        }
    }
}

/// One row in the Tunnels pane (the operator↔robot circuit).
#[derive(Debug, Clone)]
pub struct TunnelRow {
    /// Robot name this tunnel reaches.
    pub peer: String,
    /// Transport name (tcp/quic/relay).
    pub transport: String,
    /// Direct vs relayed.
    pub via_relay: bool,
    /// Bytes sent by the operator over this circuit.
    pub bytes_up: u64,
    /// Bytes received from the robot.
    pub bytes_down: u64,
}

/// One row in the Policy decisions pane.
#[derive(Debug, Clone)]
pub struct DecisionRow {
    pub ts: DateTime<Utc>,
    pub robot: String,
    pub allow: bool,
    pub operator: String,
    pub capability_group: String,
    pub reason: String,
}

/// One row in the Audit tail pane.
#[derive(Debug, Clone)]
pub struct AuditRow {
    pub ts: DateTime<Utc>,
    pub robot: String,
    pub operator: String,
    pub action: String,
    pub result: String,
    pub duration_ms: i64,
}

/// Which screen is currently showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// The four-pane dashboard.
    Dashboard,
    /// Audit tail, fullscreen.
    AuditFull,
    /// Drill-down overlay for the selected peer.
    Inspect,
    /// Help overlay.
    Help,
}

/// The complete UI state.
#[derive(Debug, Clone)]
pub struct DashboardState {
    /// Peer rows in stable, sorted-by-name order.
    pub peers: Vec<PeerRow>,
    /// Active tunnels.
    pub tunnels: Vec<TunnelRow>,
    /// Bounded ring of recent policy decisions (newest last).
    pub decisions: VecDeque<DecisionRow>,
    /// Bounded ring of recent audit records (newest last).
    pub audit: VecDeque<AuditRow>,
    /// Index of the selected peer row.
    pub selected: usize,
    /// Whether the live feed is paused (frozen display).
    pub paused: bool,
    /// Active filter string, if the operator set one.
    pub filter: Option<String>,
    /// Whether the filter input line is being edited.
    pub editing_filter: bool,
    /// Current view.
    pub view: View,
    /// When the dashboard started (for the uptime readout).
    pub started_at: DateTime<Utc>,
    /// The last time ANY event arrived, for the live-heartbeat indicator.
    pub last_activity: Option<DateTime<Utc>>,
    /// A transient "feed gapped (N dropped)" notice.
    pub gap_notice: Option<(String, u64)>,
    /// The relay address the fleet routes through, if known.
    pub relay_addr: Option<String>,
    /// The active event transport (ADR-024), for the title-bar indicator.
    /// `None` until the first feed opens.
    pub feed_transport: Option<gang_libp2p::EventsTransport>,
    /// True until at least one robot is configured (drives the first-run panel).
    pub roster_empty: bool,
    /// Events buffered while paused, applied on resume.
    pending: Vec<FeedMsg>,
}

impl DashboardState {
    /// A fresh dashboard rooted at `now`, seeded with the configured `robots`
    /// (so their rows appear immediately, before the first feed byte).
    pub fn new(now: DateTime<Utc>, robots: &[String], relay_addr: Option<String>) -> Self {
        let peers = robots.iter().cloned().map(PeerRow::new).collect::<Vec<_>>();
        Self {
            roster_empty: peers.is_empty(),
            peers,
            tunnels: Vec::new(),
            decisions: VecDeque::new(),
            audit: VecDeque::new(),
            selected: 0,
            paused: false,
            filter: None,
            editing_filter: false,
            view: View::Dashboard,
            started_at: now,
            last_activity: None,
            gap_notice: None,
            relay_addr,
            feed_transport: None,
            pending: Vec::new(),
        }
    }

    /// Ingest a feed message, honouring pause. While paused, messages are
    /// buffered (not dropped) and applied in order on resume, so a paused demo
    /// GIF resumes exactly where it left off.
    pub fn ingest(&mut self, msg: FeedMsg, now: DateTime<Utc>) {
        if self.paused {
            self.pending.push(msg);
        } else {
            self.apply(msg, now);
        }
    }

    /// Toggle pause. On resume, drain the buffer.
    pub fn toggle_pause(&mut self, now: DateTime<Utc>) {
        self.paused = !self.paused;
        if !self.paused {
            let drained = std::mem::take(&mut self.pending);
            for msg in drained {
                self.apply(msg, now);
            }
        }
    }

    /// Fold one message into the state. Pure: the only clock is `now`.
    pub fn apply(&mut self, msg: FeedMsg, now: DateTime<Utc>) {
        self.last_activity = Some(now);
        match msg {
            FeedMsg::Connected { robot } => {
                let p = self.peer_mut(&robot);
                p.connected = true;
                p.conn_down = false;
                p.last_error = None;
                p.last_seen = Some(now);
            }
            FeedMsg::Disconnected { robot, reason } => {
                let p = self.peer_mut(&robot);
                p.connected = false;
                p.last_error = Some(reason);
                self.tunnels.retain(|t| t.peer != robot);
            }
            FeedMsg::Stats { robot, stats } => {
                {
                    let p = self.peer_mut(&robot);
                    p.transport = Some(stats.transport.clone());
                    p.via_relay = stats.via_relay;
                    if stats.last_rtt_ms.is_some() {
                        p.rtt_ms = stats.last_rtt_ms;
                    }
                    // A fresh stats read is direct evidence the circuit is alive
                    // now — heartbeats are only every 15s (agent
                    // HEARTBEAT_INTERVAL), so the periodic (~2s) stats sample is
                    // what keeps a healthy peer showing "live" between beats.
                    p.conn_down = false;
                    p.connected = true;
                    p.last_seen = Some(now);
                }
                self.upsert_tunnel(&robot, &stats.transport, stats.via_relay, |t| {
                    t.bytes_up = stats.bytes_sent;
                    t.bytes_down = stats.bytes_received;
                });
            }
            FeedMsg::Event { robot, event } => self.apply_event(&robot, &event, now),
            FeedMsg::Transport { transport } => self.feed_transport = Some(transport),
        }
    }

    /// Fold one [`AgentEvent`] from `robot`. Split out so reducer tests can feed
    /// raw events directly.
    pub fn apply_event(&mut self, robot: &str, ev: &AgentEvent, now: DateTime<Utc>) {
        self.last_activity = Some(now);
        match ev {
            AgentEvent::PresenceSnapshot {
                ganglion_version,
                uptime_secs,
                installed_capabilities,
                ..
            } => {
                let p = self.peer_mut(robot);
                p.version = Some(ganglion_version.clone());
                p.uptime_secs = Some(*uptime_secs);
                p.capabilities = installed_capabilities.clone();
                p.last_seen = Some(now);
                p.connected = true;
                p.conn_down = false;
            }
            AgentEvent::Heartbeat { uptime_secs, .. } => {
                let p = self.peer_mut(robot);
                p.uptime_secs = Some(*uptime_secs);
                p.last_seen = Some(now);
                p.conn_down = false;
            }
            AgentEvent::ConnectionChanged {
                transport,
                via_relay,
                state,
                ..
            } => {
                {
                    let p = self.peer_mut(robot);
                    p.last_seen = Some(now);
                    p.transport = Some(transport.clone());
                    p.via_relay = *via_relay;
                    match state {
                        ConnectionState::Up => {
                            p.conn_down = false;
                            p.connected = true;
                        }
                        ConnectionState::Down => p.conn_down = true,
                        _ => {}
                    }
                }
                match state {
                    ConnectionState::Up => {
                        self.upsert_tunnel(robot, transport, *via_relay, |_| {});
                    }
                    ConnectionState::Down => self.tunnels.retain(|t| t.peer != robot),
                    _ => {}
                }
            }
            AgentEvent::PolicyDecision {
                ts,
                operator_peer,
                capability_group,
                decision,
                reason,
                ..
            } => {
                self.peer_mut(robot).last_seen = Some(now);
                push_capped(
                    &mut self.decisions,
                    DecisionRow {
                        ts: *ts,
                        robot: robot.to_string(),
                        allow: matches!(decision, PolicyOutcome::Allow),
                        operator: short_peer(operator_peer.as_str()),
                        capability_group: capability_group.clone(),
                        reason: reason.clone(),
                    },
                );
            }
            AgentEvent::AuditAppended { record, .. } => {
                self.peer_mut(robot).last_seen = Some(now);
                let duration_ms = (record.ended_at - record.started_at).num_milliseconds();
                push_capped(
                    &mut self.audit,
                    AuditRow {
                        ts: record.ended_at,
                        robot: robot.to_string(),
                        operator: short_peer(record.operator_peer.as_str()),
                        action: format!("{} v{}", record.component_name, record.component_version),
                        result: record.exit.clone(),
                        duration_ms,
                    },
                );
            }
            AgentEvent::Gap { dropped } => {
                self.gap_notice = Some((robot.to_string(), *dropped));
            }
            _ => {}
        }
    }

    /// Find or create the peer row for `robot`, keeping rows sorted by name.
    fn peer_mut(&mut self, robot: &str) -> &mut PeerRow {
        match self.peers.iter().position(|p| p.name == robot) {
            Some(i) => &mut self.peers[i],
            None => {
                self.roster_empty = false;
                self.peers.push(PeerRow::new(robot.to_string()));
                self.peers.sort_by(|a, b| a.name.cmp(&b.name));
                let i = self.peers.iter().position(|p| p.name == robot).unwrap();
                &mut self.peers[i]
            }
        }
    }

    fn upsert_tunnel(
        &mut self,
        robot: &str,
        transport: &str,
        via_relay: bool,
        f: impl FnOnce(&mut TunnelRow),
    ) {
        match self.tunnels.iter_mut().find(|t| t.peer == robot) {
            Some(t) => {
                t.transport = transport.to_string();
                t.via_relay = via_relay;
                f(t);
            }
            None => {
                let mut t = TunnelRow {
                    peer: robot.to_string(),
                    transport: transport.to_string(),
                    via_relay,
                    bytes_up: 0,
                    bytes_down: 0,
                };
                f(&mut t);
                self.tunnels.push(t);
                self.tunnels.sort_by(|a, b| a.peer.cmp(&b.peer));
            }
        }
    }

    // --- Selection / navigation ---

    /// The currently selected peer row, if any (after filtering).
    pub fn selected_peer(&self) -> Option<&PeerRow> {
        self.visible_peers().get(self.selected).copied()
    }

    /// Peer rows passing the active filter, in display order.
    pub fn visible_peers(&self) -> Vec<&PeerRow> {
        let f = self.filter.as_deref().map(str::to_lowercase);
        self.peers
            .iter()
            .filter(|p| match &f {
                None => true,
                Some(needle) => {
                    p.name.to_lowercase().contains(needle)
                        || p.transport
                            .as_deref()
                            .is_some_and(|t| t.to_lowercase().contains(needle))
                }
            })
            .collect()
    }

    /// Move the peer selection by `delta`, clamped to the visible rows.
    pub fn move_selection(&mut self, delta: isize) {
        let n = self.visible_peers().len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected.min(n - 1) as isize;
        let next = (cur + delta).rem_euclid(n as isize);
        self.selected = next as usize;
    }

    /// Decisions passing the active filter (by robot, capability, or reason).
    pub fn visible_decisions(&self) -> Vec<&DecisionRow> {
        let f = self.filter.as_deref().map(str::to_lowercase);
        self.decisions
            .iter()
            .filter(|d| match &f {
                None => true,
                Some(n) => {
                    d.robot.to_lowercase().contains(n)
                        || d.capability_group.to_lowercase().contains(n)
                        || d.reason.to_lowercase().contains(n)
                        || d.operator.to_lowercase().contains(n)
                }
            })
            .collect()
    }

    /// Audit rows passing the active filter.
    pub fn visible_audit(&self) -> Vec<&AuditRow> {
        let f = self.filter.as_deref().map(str::to_lowercase);
        self.audit
            .iter()
            .filter(|a| match &f {
                None => true,
                Some(n) => {
                    a.robot.to_lowercase().contains(n)
                        || a.action.to_lowercase().contains(n)
                        || a.result.to_lowercase().contains(n)
                        || a.operator.to_lowercase().contains(n)
                }
            })
            .collect()
    }

    /// Count of peers currently live at `now`.
    pub fn live_count(&self, now: DateTime<Utc>) -> usize {
        self.peers
            .iter()
            .filter(|p| p.status(now) == PeerStatus::Live)
            .count()
    }

    /// Seconds since the dashboard started.
    pub fn uptime_secs(&self, now: DateTime<Utc>) -> u64 {
        (now - self.started_at).num_seconds().max(0) as u64
    }

    /// A short label for the active event transport (ADR-024), for the title
    /// bar: `push`, `poll(1.5s)`, or empty until the first feed opens.
    pub fn feed_transport_label(&self) -> String {
        match self.feed_transport {
            Some(gang_libp2p::EventsTransport::Poll) => "poll(1.5s)".to_string(),
            Some(_) => "push".to_string(),
            None => String::new(),
        }
    }

    /// Whether the feed looks stale: no activity (event or ~2s stats sample)
    /// within 4.5s. The event feed is push (instant), so staleness reflects a
    /// dead/stalled connection rather than a missed poll.
    pub fn feed_stale(&self, now: DateTime<Utc>) -> bool {
        match self.last_activity {
            None => true,
            Some(t) => (now - t).num_milliseconds() > 4_500,
        }
    }
}

/// Push onto a bounded ring, evicting the oldest past [`TAIL_CAP`].
fn push_capped<T>(ring: &mut VecDeque<T>, item: T) {
    if ring.len() >= TAIL_CAP {
        ring.pop_front();
    }
    ring.push_back(item);
}

/// Abbreviate a peer id for a compact cell. Uses ASCII `..` so the string is
/// identical in the colour and `NO_COLOR`/ASCII themes.
pub fn short_peer(s: &str) -> String {
    if s.len() > 13 {
        format!("{}..", &s[..13])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use gang_core::events::AuditProjection;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap()
    }

    fn peer_id() -> gang_core::identity::PeerId {
        gang_core::identity::Keypair::generate().peer_id()
    }

    /// Build an [`AuditProjection`] (which is `#[non_exhaustive]`) via the
    /// public `From<&AuditRecord>` path, since gang-cli cannot use its struct
    /// literal.
    fn projection(
        op: gang_core::identity::PeerId,
        exit: &str,
        started: DateTime<Utc>,
        ended: DateTime<Utc>,
    ) -> AuditProjection {
        use gang_core::audit::{AuditRecord, ExitStatus};
        let status = match exit {
            "success" => ExitStatus::Success,
            other => ExitStatus::Failed {
                message: other.into(),
            },
        };
        let rec = AuditRecord {
            operator_peer_id: op,
            component_name: "diagnostics".into(),
            component_version: "0.1.0".into(),
            component_hash: "abc".into(),
            capabilities_used: vec!["ganglion:diagnostics/collect".into()],
            started_at: started,
            ended_at: ended,
            exit_status: status,
            io_stats: vec![],
        };
        (&rec).into()
    }

    #[test]
    fn presence_snapshot_populates_peer_row() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        s.apply_event(
            "up-robot",
            &AgentEvent::PresenceSnapshot {
                seq: 1,
                ganglion_version: "2.1.0".into(),
                uptime_secs: 300,
                archetype: Some("nat-office".into()),
                installed_capabilities: vec!["diagnostics".into()],
            },
            t0(),
        );
        let p = &s.peers[0];
        assert_eq!(p.version.as_deref(), Some("2.1.0"));
        assert_eq!(p.capabilities, vec!["diagnostics".to_string()]);
        assert_eq!(p.status(t0()), PeerStatus::Live);
    }

    #[test]
    fn policy_decisions_land_in_the_decision_pane() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        for (grp, outcome, reason) in [
            (
                "ganglion:diagnostics/collect",
                PolicyOutcome::Allow,
                "declared + trusted",
            ),
            (
                "ganglion:process/spawn",
                PolicyOutcome::Deny,
                "capability not declared",
            ),
        ] {
            s.apply_event(
                "up-robot",
                &AgentEvent::PolicyDecision {
                    seq: 2,
                    ts: t0(),
                    operator_peer: peer_id(),
                    capability_group: grp.into(),
                    decision: outcome,
                    reason: reason.into(),
                },
                t0(),
            );
        }
        assert_eq!(s.decisions.len(), 2);
        assert!(s.decisions[0].allow);
        assert!(!s.decisions[1].allow);
        assert_eq!(s.decisions[1].capability_group, "ganglion:process/spawn");
    }

    #[test]
    fn audit_appended_records_duration() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        let start = t0();
        let end = start + chrono::Duration::milliseconds(1234);
        let rec = projection(peer_id(), "success", start, end);
        s.apply_event(
            "up-robot",
            &AgentEvent::AuditAppended {
                seq: 3,
                record: rec,
            },
            t0(),
        );
        assert_eq!(s.audit.len(), 1);
        assert_eq!(s.audit[0].duration_ms, 1234);
        assert_eq!(s.audit[0].result, "success");
    }

    #[test]
    fn connection_up_creates_tunnel_down_removes_it() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        s.apply_event(
            "up-robot",
            &AgentEvent::ConnectionChanged {
                seq: 4,
                ts: t0(),
                peer: peer_id(),
                transport: "quic".into(),
                via_relay: true,
                state: ConnectionState::Up,
            },
            t0(),
        );
        assert_eq!(s.tunnels.len(), 1);
        assert!(s.tunnels[0].via_relay);
        s.apply_event(
            "up-robot",
            &AgentEvent::ConnectionChanged {
                seq: 5,
                ts: t0(),
                peer: peer_id(),
                transport: "quic".into(),
                via_relay: true,
                state: ConnectionState::Down,
            },
            t0(),
        );
        assert!(s.tunnels.is_empty());
        assert_eq!(s.peers[0].status(t0()), PeerStatus::Offline);
    }

    #[test]
    fn stats_update_byte_counters_and_rtt() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        let mut stats = TransportStats {
            transport: "quic".into(),
            via_relay: true,
            bytes_sent: 2048,
            bytes_received: 4096,
            last_rtt_ms: Some(42),
            ..Default::default()
        };
        s.apply(
            FeedMsg::Stats {
                robot: "up-robot".into(),
                stats: stats.clone(),
            },
            t0(),
        );
        assert_eq!(s.tunnels[0].bytes_up, 2048);
        assert_eq!(s.tunnels[0].bytes_down, 4096);
        assert_eq!(s.peers[0].rtt_ms, Some(42));
        // A later stats poll with no RTT must not wipe the last good RTT.
        stats.last_rtt_ms = None;
        stats.bytes_sent = 3000;
        s.apply(
            FeedMsg::Stats {
                robot: "up-robot".into(),
                stats,
            },
            t0(),
        );
        assert_eq!(s.peers[0].rtt_ms, Some(42));
        assert_eq!(s.tunnels[0].bytes_up, 3000);
    }

    #[test]
    fn liveness_decays_with_time() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        s.apply(
            FeedMsg::Connected {
                robot: "up-robot".into(),
            },
            t0(),
        );
        assert_eq!(s.peers[0].status(t0()), PeerStatus::Live);
        let later = t0() + chrono::Duration::seconds(8);
        assert_eq!(s.peers[0].status(later), PeerStatus::Transitional);
        let much_later = t0() + chrono::Duration::seconds(30);
        assert_eq!(s.peers[0].status(much_later), PeerStatus::Offline);
    }

    #[test]
    fn pause_buffers_then_resume_applies() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        s.toggle_pause(t0());
        assert!(s.paused);
        s.ingest(
            FeedMsg::Event {
                robot: "up-robot".into(),
                event: AgentEvent::PolicyDecision {
                    seq: 1,
                    ts: t0(),
                    operator_peer: peer_id(),
                    capability_group: "x".into(),
                    decision: PolicyOutcome::Deny,
                    reason: "r".into(),
                },
            },
            t0(),
        );
        // Nothing applied while paused.
        assert_eq!(s.decisions.len(), 0);
        s.toggle_pause(t0());
        // Resume drains the buffer.
        assert_eq!(s.decisions.len(), 1);
    }

    #[test]
    fn gap_event_sets_notice() {
        let mut s = DashboardState::new(t0(), &["up-robot".into()], None);
        s.apply_event("up-robot", &AgentEvent::Gap { dropped: 7 }, t0());
        assert_eq!(s.gap_notice, Some(("up-robot".into(), 7)));
    }

    #[test]
    fn filter_narrows_visible_peers() {
        let mut s = DashboardState::new(t0(), &["alpha".into(), "beta".into()], None);
        s.filter = Some("alph".into());
        assert_eq!(s.visible_peers().len(), 1);
        assert_eq!(s.visible_peers()[0].name, "alpha");
    }

    #[test]
    fn selection_wraps_within_visible_rows() {
        let mut s = DashboardState::new(t0(), &["a".into(), "b".into(), "c".into()], None);
        assert_eq!(s.selected, 0);
        s.move_selection(-1);
        assert_eq!(s.selected, 2); // wrap to last
        s.move_selection(1);
        assert_eq!(s.selected, 0); // wrap to first
    }

    #[test]
    fn unknown_robot_creates_row_and_clears_empty_flag() {
        let mut s = DashboardState::new(t0(), &[], None);
        assert!(s.roster_empty);
        s.apply(
            FeedMsg::Connected {
                robot: "surprise".into(),
            },
            t0(),
        );
        assert!(!s.roster_empty);
        assert_eq!(s.peers.len(), 1);
    }
}
