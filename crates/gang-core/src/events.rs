//! The robot→operator event subscription wire model.
//!
//! Ganglion exposes a long-lived, authenticated event feed on
//! [`crate::protocol::PROTOCOL_EVENTS`] (`/ganglion/events/1.0`). A subscriber
//! sends a single [`EventSubscribeRequest`] and receives a length-prefixed CBOR
//! sequence of [`AgentEvent`]s, framed with the SAME codec used for control
//! messages ([`crate::message::encode_message`] / [`decode_message`]).
//!
//! # What this module is (and is not)
//!
//! This module is pure data: the wire enum, the request, and the framing
//! helpers. It holds no transport, no keys, and no policy — the robot-side
//! event bus, emission sites, and subscriber authentication live in
//! `gang-ros`; the operator-side open/decode lives in `gang-libp2p`.
//!
//! # No secrets on the wire
//!
//! Every [`AgentEvent`] variant is constrained to non-secret material:
//! identities (public peer ids), capability-group names, policy verdicts and
//! human-readable reasons, and byte/timing counters. There is deliberately no
//! variant carrying private keys, pairing-token secrets, component bytes, or
//! captured tool output. A round-trip test in this module asserts the CBOR of a
//! representative event contains none of a set of secret markers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::audit::{AuditRecord, ExitStatus};
use crate::identity::PeerId;
use crate::message::{DecodeError, decode_message, encode_message};

/// Monotonic, per-agent sequence number stamped on every emitted event.
///
/// Sequence numbers let a subscriber resume with `since_seq` (see
/// [`EventSubscribeRequest`]) and let the robot report a [`AgentEvent::Gap`]
/// when the subscriber fell behind the retained window.
pub type EventSeq = u64;

/// A subscription request, sent once by an operator when opening
/// `/ganglion/events/1.0`.
///
/// The request is CBOR + length-prefixed exactly like a control message. An
/// empty stream (no bytes) is treated as [`EventSubscribeRequest::default`],
/// i.e. "send me the presence snapshot and all retained recent events".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EventSubscribeRequest {
    /// Return only events with `seq` strictly greater than this cursor.
    ///
    /// `None` requests a fresh subscription: the robot replies with a
    /// [`AgentEvent::PresenceSnapshot`] followed by every retained recent
    /// event. A polling subscriber advances this cursor across calls using
    /// [`AgentEvent::seq`] of the last event it saw.
    #[serde(default)]
    pub since_seq: Option<EventSeq>,

    /// Soft cap on the number of events returned in this batch. The robot
    /// clamps this to its own retention window; `None` means "the robot's
    /// default window".
    #[serde(default)]
    pub max_events: Option<u32>,
}

impl EventSubscribeRequest {
    /// A fresh subscription (snapshot + all retained recent events).
    pub fn fresh() -> Self {
        Self::default()
    }

    /// A resume request returning only events newer than `cursor`.
    pub fn since(cursor: EventSeq) -> Self {
        Self {
            since_seq: Some(cursor),
            max_events: None,
        }
    }
}

/// The verdict of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PolicyOutcome {
    /// The declared capabilities were permitted.
    Allow,
    /// The request was denied by policy.
    Deny,
}

/// Direction of a connection-state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConnectionState {
    /// A peer connection was established.
    Up,
    /// A peer connection was lost.
    Down,
}

/// A secret-free projection of an [`AuditRecord`] suitable for streaming.
///
/// The full [`AuditRecord`] already contains no secret material, but this
/// projection is intentionally narrow: it carries only what a live viewer or
/// `gang logs` line needs, so the wire surface cannot accidentally grow to
/// include sensitive fields in a future record revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditProjection {
    /// The operator whose invocation produced this record.
    pub operator_peer: PeerId,
    /// The component that ran.
    pub component_name: String,
    /// The component version.
    pub component_version: String,
    /// Capability groups the invocation used.
    pub capabilities_used: Vec<String>,
    /// A short, human-readable terminal status (`success`, `policy_denied`, …).
    pub exit: String,
    /// Wall-clock start.
    pub started_at: DateTime<Utc>,
    /// Wall-clock end.
    pub ended_at: DateTime<Utc>,
}

impl From<&AuditRecord> for AuditProjection {
    fn from(r: &AuditRecord) -> Self {
        Self {
            operator_peer: r.operator_peer_id.clone(),
            component_name: r.component_name.clone(),
            component_version: r.component_version.clone(),
            capabilities_used: r.capabilities_used.clone(),
            exit: exit_status_label(&r.exit_status).to_string(),
            started_at: r.started_at,
            ended_at: r.ended_at,
        }
    }
}

/// A stable, short label for an [`ExitStatus`], for display and JSONL.
pub fn exit_status_label(status: &ExitStatus) -> &'static str {
    match status {
        ExitStatus::Success => "success",
        ExitStatus::Failed { .. } => "failed",
        ExitStatus::Timeout => "timeout",
        ExitStatus::Trapped { .. } => "trapped",
        ExitStatus::PolicyDenied { .. } => "policy_denied",
    }
}

/// An event emitted by a robot agent to authenticated subscribers.
///
/// Versioned and `#[non_exhaustive]`: new variants and fields may be added in a
/// backward-compatible way. Every streamed variant carries a monotonic `seq`
/// (see [`AgentEvent::seq`]); [`AgentEvent::Gap`] is a synthetic marker with no
/// sequence of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentEvent {
    /// Sent once at the head of a fresh subscription: a point-in-time view of
    /// the agent.
    PresenceSnapshot {
        /// Sequence at the time the snapshot was taken (the current tip).
        seq: EventSeq,
        /// The agent's Ganglion version string.
        ganglion_version: String,
        /// Agent uptime in seconds.
        uptime_secs: u64,
        /// Detected network archetype, if known.
        archetype: Option<String>,
        /// Names of installed capabilities.
        installed_capabilities: Vec<String>,
    },
    /// A policy evaluation occurred (both allow and deny paths are emitted).
    PolicyDecision {
        /// Monotonic sequence.
        seq: EventSeq,
        /// When the decision was made.
        ts: DateTime<Utc>,
        /// The operator whose request was evaluated.
        operator_peer: PeerId,
        /// The capability group(s) evaluated (comma-joined for a bundle).
        capability_group: String,
        /// The verdict.
        decision: PolicyOutcome,
        /// Human-readable rationale (never secret material).
        reason: String,
    },
    /// A record was appended to the robot's audit log.
    AuditAppended {
        /// Monotonic sequence.
        seq: EventSeq,
        /// The secret-free projection of the appended record.
        record: AuditProjection,
    },
    /// A peer connection came up or went down.
    ConnectionChanged {
        /// Monotonic sequence.
        seq: EventSeq,
        /// When the change was observed.
        ts: DateTime<Utc>,
        /// The peer whose connection changed.
        peer: PeerId,
        /// The transport in use (tcp, quic, relay, …).
        transport: String,
        /// Whether the connection is relayed.
        via_relay: bool,
        /// Whether the connection came up or went down.
        state: ConnectionState,
    },
    /// A periodic liveness beat, so a viewer can distinguish "quiet" from
    /// "disconnected".
    Heartbeat {
        /// Monotonic sequence.
        seq: EventSeq,
        /// When the beat was emitted.
        ts: DateTime<Utc>,
        /// Agent uptime in seconds.
        uptime_secs: u64,
    },
    /// A marker inserted when the subscriber fell behind: `dropped` events were
    /// evicted from the retained window (slow-consumer / lag path) before they
    /// could be delivered. The subscriber resumes from the next retained event.
    Gap {
        /// Number of events that were dropped before this marker.
        dropped: u64,
    },
}

impl AgentEvent {
    /// The sequence number carried by this event, or `None` for a
    /// [`AgentEvent::Gap`] marker (which has no sequence of its own).
    pub fn seq(&self) -> Option<EventSeq> {
        match self {
            AgentEvent::PresenceSnapshot { seq, .. }
            | AgentEvent::PolicyDecision { seq, .. }
            | AgentEvent::AuditAppended { seq, .. }
            | AgentEvent::ConnectionChanged { seq, .. }
            | AgentEvent::Heartbeat { seq, .. } => Some(*seq),
            AgentEvent::Gap { .. } => None,
        }
    }
}

/// Encode a batch of events as a concatenation of length-prefixed CBOR frames,
/// reusing the control-message framing ([`encode_message`]). This is the exact
/// byte layout a subscriber decodes with [`decode_events`].
pub fn encode_events(
    events: &[AgentEvent],
) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut out = Vec::new();
    for ev in events {
        out.extend_from_slice(&encode_message(ev)?);
    }
    Ok(out)
}

/// Decode a batch of events from a buffer of concatenated length-prefixed CBOR
/// frames. Stops at the first incomplete trailing frame (returning what decoded
/// cleanly) and surfaces a genuine CBOR error otherwise.
pub fn decode_events(mut data: &[u8]) -> Result<Vec<AgentEvent>, DecodeError> {
    let mut events = Vec::new();
    while !data.is_empty() {
        match decode_message::<AgentEvent>(data) {
            Ok((ev, consumed)) => {
                events.push(ev);
                data = &data[consumed..];
            }
            // A partial trailing frame is not an error for a best-effort batch
            // reader — return everything decoded so far.
            Err(DecodeError::Incomplete) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_events() -> Vec<AgentEvent> {
        let peer = crate::identity::Keypair::generate().peer_id();
        vec![
            AgentEvent::PresenceSnapshot {
                seq: 3,
                ganglion_version: "2.1.0".into(),
                uptime_secs: 42,
                archetype: Some("nat-office".into()),
                installed_capabilities: vec!["diagnostics".into()],
            },
            AgentEvent::PolicyDecision {
                seq: 4,
                ts: Utc::now(),
                operator_peer: peer.clone(),
                capability_group: "ganglion:process/spawn".into(),
                decision: PolicyOutcome::Deny,
                reason: "capability denied by policy".into(),
            },
            AgentEvent::Gap { dropped: 7 },
            AgentEvent::Heartbeat {
                seq: 5,
                ts: Utc::now(),
                uptime_secs: 43,
            },
        ]
    }

    #[test]
    fn events_batch_roundtrips() {
        let events = sample_events();
        let encoded = encode_events(&events).unwrap();
        let decoded = decode_events(&encoded).unwrap();
        assert_eq!(decoded.len(), events.len());
        assert_eq!(decoded[0].seq(), Some(3));
        assert_eq!(decoded[2].seq(), None); // Gap
        assert!(matches!(decoded[1], AgentEvent::PolicyDecision { .. }));
    }

    #[test]
    fn partial_trailing_frame_is_tolerated() {
        let events = sample_events();
        let mut encoded = encode_events(&events).unwrap();
        // Chop the last few bytes: the final frame is now incomplete.
        encoded.truncate(encoded.len() - 2);
        let decoded = decode_events(&encoded).unwrap();
        // Everything but the last event still decodes.
        assert_eq!(decoded.len(), events.len() - 1);
    }

    #[test]
    fn empty_request_default_is_fresh() {
        let req = EventSubscribeRequest::default();
        assert!(req.since_seq.is_none());
        let bytes = encode_message(&req).unwrap();
        let (back, _): (EventSubscribeRequest, usize) = decode_message(&bytes).unwrap();
        assert!(back.since_seq.is_none());
    }

    #[test]
    fn events_carry_no_secret_material() {
        // A representative event that references an operator identity and an
        // audit projection must not serialize any secret markers. This guards
        // against a future variant leaking key/token material onto the wire.
        let kp = crate::identity::Keypair::generate();
        let record = AuditRecord {
            operator_peer_id: kp.peer_id(),
            component_name: "diagnostics".into(),
            component_version: "0.1.0".into(),
            component_hash: "abc".into(),
            capabilities_used: vec!["ganglion:diagnostics/collect".into()],
            started_at: Utc::now(),
            ended_at: Utc::now(),
            exit_status: ExitStatus::Success,
            io_stats: vec![],
        };
        let ev = AgentEvent::AuditAppended {
            seq: 1,
            record: (&record).into(),
        };
        let bytes = encode_message(&ev).unwrap();

        // A signature is derived from the private key; it is never part of an
        // event, so its bytes must not appear. (The operator's PUBLIC peer id
        // is expected to appear and is not secret.)
        let sig = kp.sign(b"probe").to_bytes();
        assert!(
            !contains_subsequence(&bytes, &sig),
            "event serialization leaked private-key-derived bytes"
        );
        // Nor any of these textual secret markers.
        let haystack = String::from_utf8_lossy(&bytes);
        for marker in ["token_secret", "private", "secret_key", "BEGIN"] {
            assert!(
                !haystack.contains(marker),
                "event serialization contains secret marker {marker:?}"
            );
        }
    }

    fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        if needle.is_empty() || haystack.len() < needle.len() {
            return false;
        }
        haystack.windows(needle.len()).any(|w| w == needle)
    }
}
