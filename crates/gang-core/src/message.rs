use serde::{Deserialize, Serialize};

use crate::identity::PeerId;

/// Length-prefixed CBOR framing for Ganglion messages.
/// Wire format: [varint length][CBOR payload]
///
/// Encode a message to length-prefixed CBOR bytes.
pub fn encode_message<T: Serialize>(
    msg: &T,
) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut payload = Vec::new();
    ciborium::into_writer(msg, &mut payload)?;

    let mut frame = Vec::with_capacity(payload.len() + 4);
    write_varint(&mut frame, payload.len() as u64);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a length-prefixed CBOR message from a byte slice.
/// Returns the decoded message and the number of bytes consumed.
pub fn decode_message<T: for<'de> Deserialize<'de>>(
    data: &[u8],
) -> Result<(T, usize), DecodeError> {
    let (len, varint_size) = read_varint(data).ok_or(DecodeError::Incomplete)?;
    let total = varint_size + len as usize;

    if data.len() < total {
        return Err(DecodeError::Incomplete);
    }

    let payload = &data[varint_size..total];
    let msg: T =
        ciborium::from_reader(payload).map_err(|e| DecodeError::CborError(e.to_string()))?;
    Ok((msg, total))
}

/// Errors returned when decoding a length-prefixed message frame.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The buffer does not yet contain a complete frame.
    #[error("incomplete frame, need more data")]
    Incomplete,
    /// The CBOR payload failed to decode.
    #[error("cbor decode: {0}")]
    CborError(String),
}

fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value > 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

// --- Control protocol messages ---

/// Messages on /ganglion/control/1.0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlMessage {
    /// Presence announcement from a robot agent.
    Presence {
        /// The announcing peer's ID.
        peer_id: PeerId,
        /// Names of installed capabilities.
        capabilities: Vec<String>,
        /// Agent uptime in seconds.
        uptime_secs: u64,
        /// Ganglion version string.
        version: String,
    },
    /// Deploy a signed capability to a robot.
    DeployCapability {
        /// Capability name.
        name: String,
        /// Capability version.
        version: String,
        /// CBOR-encoded signed manifest.
        manifest_cbor: Vec<u8>,
        /// The WASM component bytes.
        component_bytes: Vec<u8>,
        /// Unique per-request nonce for replay protection (v1.1, additive).
        /// Empty on legacy messages predating replay protection.
        #[serde(default)]
        nonce: String,
        /// Unix time in milliseconds when the request was created (v1.1,
        /// additive). Zero on legacy messages.
        #[serde(default)]
        timestamp_ms: u64,
    },
    /// Invoke an installed capability.
    InvokeCapability {
        /// Capability name.
        name: String,
        /// Invocation arguments.
        args: Vec<String>,
        /// Correlation ID matching the eventual `InvokeResult`.
        request_id: String,
        /// Unique per-request nonce for replay protection (v1.1, additive).
        /// Empty on legacy messages predating replay protection.
        #[serde(default)]
        nonce: String,
        /// Unix time in milliseconds when the request was created (v1.1,
        /// additive). Zero on legacy messages.
        #[serde(default)]
        timestamp_ms: u64,
    },
    /// Response to a capability invocation.
    InvokeResult {
        /// Correlation ID of the originating request.
        request_id: String,
        /// Terminal status of the invocation.
        status: InvokeStatus,
        /// Captured output bytes.
        output: Vec<u8>,
    },
    /// List installed capabilities (request).
    ListCapabilities,
    /// List installed capabilities (response).
    CapabilityList {
        /// The installed capabilities.
        capabilities: Vec<CapabilityInfo>,
    },
    /// Robot → operator enrollment request over a pairing session (`gang join`).
    ///
    /// Rides the control protocol so it reuses the same authenticated circuit
    /// the rest of the fleet uses. The operator authenticates the *robot* from
    /// the wire (the Ed25519 key libp2p proved on this stream), never from this
    /// message — `libp2p_id` here is only a convenience the operator accepts
    /// solely after confirming its embedded key derives to the wire-authenticated
    /// gang id. `token_secret` is the single-use bearer secret from the pairing
    /// token; it proves the robot was invited.
    Enroll {
        /// The pairing token's bearer secret (see `gang_core::pairing`).
        token_secret: Vec<u8>,
        /// The robot's requested registry name.
        name: String,
        /// The robot's self-reported dialable base58 libp2p id. Accepted only
        /// when its embedded key derives to the wire-authenticated gang id.
        libp2p_id: String,
    },
    /// Operator → robot enrollment acknowledgement.
    ///
    /// Lets the robot confirm which operator identity recorded it (the robot
    /// already cryptographically reached the operator its token named; this
    /// echoes it back for a clear success message).
    Enrolled {
        /// The operator's gang id that recorded the robot.
        operator_id: PeerId,
        /// The robot's gang id as recorded (the wire-authenticated one).
        robot_id: PeerId,
        /// The name the robot was registered under.
        name: String,
    },
    /// Error response.
    Error {
        /// Correlation ID of the request that failed, if any.
        request_id: Option<String>,
        /// Machine-readable error code.
        code: String,
        /// Human-readable error message.
        message: String,
    },
}

/// Terminal status of a capability invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InvokeStatus {
    /// The invocation completed successfully.
    Success,
    /// The invocation failed.
    Failed,
    /// The invocation exceeded its deadline.
    Timeout,
    /// The invocation was denied by policy.
    PolicyDenied,
    /// The WASM guest trapped.
    Trapped,
}

/// Summary of an installed capability, returned in `CapabilityList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityInfo {
    /// Capability name.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// Author peer ID.
    pub author: PeerId,
    /// Declared capability group names.
    pub declared_capabilities: Vec<String>,
}

impl ControlMessage {
    /// Return the replay-protection metadata `(nonce, timestamp_ms)` for
    /// request messages that carry it (`DeployCapability`, `InvokeCapability`).
    /// Returns `None` for messages that are not authenticated requests.
    pub fn request_meta(&self) -> Option<(&str, u64)> {
        match self {
            ControlMessage::DeployCapability {
                nonce,
                timestamp_ms,
                ..
            }
            | ControlMessage::InvokeCapability {
                nonce,
                timestamp_ms,
                ..
            } => Some((nonce.as_str(), *timestamp_ms)),
            _ => None,
        }
    }
}

/// Generate a fresh, unique request nonce (UUID v4, hyphenated hex).
pub fn fresh_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Current Unix time in milliseconds, for stamping outgoing requests.
pub fn unix_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether a request timestamp is fresh relative to `now_ms` within `window`.
///
/// A request is fresh when `|now_ms - timestamp_ms| <= window`. The symmetric
/// bound tolerates modest clock skew in either direction.
pub fn is_fresh(timestamp_ms: u64, now_ms: u64, window: std::time::Duration) -> bool {
    let window_ms = window.as_millis() as u64;
    let delta = now_ms.abs_diff(timestamp_ms);
    delta <= window_ms
}

/// Errors returned when a request fails replay validation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReplayError {
    /// The request nonce was empty (not populated by the sender).
    #[error("request is missing a replay nonce")]
    MissingNonce,
    /// The request timestamp falls outside the freshness window.
    #[error("request timestamp is outside the freshness window")]
    Stale,
    /// The nonce has already been observed within the window.
    #[error("request nonce has already been seen (replay)")]
    Replay,
    /// The guard is tracking its maximum number of nonces; new requests are
    /// rejected (fail closed) until entries age out of the window.
    #[error("replay guard is at capacity; rejecting request (fail closed)")]
    CapacityExhausted,
}

/// Tracks recently-seen request nonces to reject replays, evicting entries
/// once they age past the freshness window.
///
/// The enforcement wiring (calling [`ReplayGuard::observe`] on each inbound
/// request) lives in the agent/transport layers; this type just provides the
/// validation primitive and its state.
#[derive(Debug)]
pub struct ReplayGuard {
    window: std::time::Duration,
    /// Map of seen nonce -> timestamp_ms it was seen at.
    seen: std::collections::HashMap<String, u64>,
    /// Hard cap on tracked nonces (memory bound).
    max_tracked: usize,
}

/// Default hard cap on the number of nonces a [`ReplayGuard`] tracks.
///
/// Without a cap, an attacker could grow the seen-nonce map without bound by
/// sending unique fresh nonces inside the freshness window. When the guard is
/// full (after evicting aged entries), new requests are *rejected* — failing
/// closed — rather than accepted untracked, which would reopen the replay
/// window.
pub const MAX_SEEN: usize = 100_000;

impl ReplayGuard {
    /// Create a guard that accepts requests whose timestamp is within `window`
    /// of the current time and whose nonce has not been seen. Tracks at most
    /// [`MAX_SEEN`] nonces; see [`ReplayGuard::with_max_tracked`].
    pub fn new(window: std::time::Duration) -> Self {
        Self::with_max_tracked(window, MAX_SEEN)
    }

    /// Like [`ReplayGuard::new`] but with an explicit cap on tracked nonces.
    /// Once `max_tracked` nonces are retained (and none have aged out), new
    /// requests fail with [`ReplayError::CapacityExhausted`].
    pub fn with_max_tracked(window: std::time::Duration, max_tracked: usize) -> Self {
        Self {
            window,
            seen: std::collections::HashMap::new(),
            max_tracked,
        }
    }

    /// The configured freshness window.
    pub fn window(&self) -> std::time::Duration {
        self.window
    }

    /// Validate and record a request's replay metadata against the current
    /// wall clock. Returns `Ok(())` if the request is fresh and previously
    /// unseen; the nonce is then remembered for the duration of the window.
    pub fn observe(&mut self, nonce: &str, timestamp_ms: u64) -> Result<(), ReplayError> {
        self.observe_at(nonce, timestamp_ms, unix_millis_now())
    }

    /// Like [`ReplayGuard::observe`] but with an explicit `now_ms` (for tests
    /// and deterministic callers).
    pub fn observe_at(
        &mut self,
        nonce: &str,
        timestamp_ms: u64,
        now_ms: u64,
    ) -> Result<(), ReplayError> {
        if nonce.is_empty() {
            return Err(ReplayError::MissingNonce);
        }
        // Drop entries that have aged out of the window.
        self.evict_expired(now_ms);

        if !is_fresh(timestamp_ms, now_ms, self.window) {
            return Err(ReplayError::Stale);
        }
        if self.seen.contains_key(nonce) {
            return Err(ReplayError::Replay);
        }
        // Hard memory bound: when full even after eviction, reject rather
        // than accept an untracked nonce (fail closed — accepting would let
        // the same nonce be replayed while the guard is saturated).
        if self.seen.len() >= self.max_tracked {
            return Err(ReplayError::CapacityExhausted);
        }
        self.seen.insert(nonce.to_string(), timestamp_ms);
        Ok(())
    }

    /// Number of nonces currently retained.
    pub fn tracked(&self) -> usize {
        self.seen.len()
    }

    fn evict_expired(&mut self, now_ms: u64) {
        let window = self.window;
        self.seen.retain(|_, &mut ts| is_fresh(ts, now_ms, window));
    }
}

// --- Tool protocol messages ---

/// Messages on /ganglion/tool/1.0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolMessage {
    /// Data chunk from capability to operator (or vice versa).
    Data {
        /// The chunk payload.
        payload: Vec<u8>,
    },
    /// End of stream.
    Eof,
    /// Error during tool execution.
    Error {
        /// Human-readable error message.
        message: String,
    },
}

// --- Bulk transfer messages ---

/// Messages on /ganglion/bulk/1.0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkMessage {
    /// Offer an artifact for transfer.
    Offer {
        /// Artifact name.
        name: String,
        /// Total artifact size in bytes.
        size: u64,
        /// Content hash of the artifact.
        hash: String,
    },
    /// Accept the offered artifact.
    Accept,
    /// A data chunk.
    Chunk {
        /// Byte offset of this chunk within the artifact.
        offset: u64,
        /// The chunk bytes.
        data: Vec<u8>,
    },
    /// Transfer complete.
    Complete {
        /// Content hash of the fully-received artifact.
        hash: String,
    },
    /// Progress report.
    Progress {
        /// Bytes transferred so far.
        bytes_transferred: u64,
        /// Total bytes expected.
        total_bytes: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for value in [0u64, 1, 127, 128, 255, 16384, u64::MAX / 2] {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            let (decoded, size) = read_varint(&buf).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(size, buf.len());
        }
    }

    #[test]
    fn message_roundtrip() {
        let msg = ControlMessage::Presence {
            peer_id: crate::identity::PeerId::from_public_key(
                &crate::identity::Keypair::generate().public_key(),
            ),
            capabilities: vec!["diagnostics".into()],
            uptime_secs: 3600,
            version: "0.1.0".into(),
        };

        let encoded = encode_message(&msg).unwrap();
        let (decoded, consumed): (ControlMessage, usize) = decode_message(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());

        match decoded {
            ControlMessage::Presence { uptime_secs, .. } => {
                assert_eq!(uptime_secs, 3600);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn request_meta_exposes_nonce_and_timestamp() {
        let msg = ControlMessage::InvokeCapability {
            name: "diagnostics".into(),
            args: vec![],
            request_id: "r1".into(),
            nonce: "abc".into(),
            timestamp_ms: 42,
        };
        assert_eq!(msg.request_meta(), Some(("abc", 42)));

        // Non-request messages have no replay metadata.
        assert_eq!(ControlMessage::ListCapabilities.request_meta(), None);
    }

    #[test]
    fn legacy_message_without_nonce_deserializes() {
        // A message missing the new fields still decodes (additive/backward
        // compatible), with empty nonce and zero timestamp.
        let json = serde_json::json!({
            "type": "invoke_capability",
            "name": "diagnostics",
            "args": [],
            "request_id": "r1",
        });
        let msg: ControlMessage = serde_json::from_value(json).unwrap();
        assert_eq!(msg.request_meta(), Some(("", 0)));
    }

    #[test]
    fn freshness_window() {
        let now = 1_000_000u64;
        let window = std::time::Duration::from_secs(30);
        assert!(is_fresh(now, now, window));
        assert!(is_fresh(now - 29_000, now, window));
        assert!(is_fresh(now + 29_000, now, window));
        assert!(!is_fresh(now - 31_000, now, window));
        assert!(!is_fresh(now + 31_000, now, window));
    }

    #[test]
    fn replay_guard_rejects_replays_and_stale() {
        let mut guard = ReplayGuard::new(std::time::Duration::from_secs(30));
        let now = 1_000_000u64;

        // Missing nonce.
        assert_eq!(
            guard.observe_at("", now, now),
            Err(ReplayError::MissingNonce)
        );

        // Fresh, unseen nonce is accepted.
        assert_eq!(guard.observe_at("n1", now, now), Ok(()));
        assert_eq!(guard.tracked(), 1);

        // Same nonce again -> replay.
        assert_eq!(guard.observe_at("n1", now, now), Err(ReplayError::Replay));

        // Stale timestamp -> rejected.
        assert_eq!(
            guard.observe_at("n2", now - 60_000, now),
            Err(ReplayError::Stale)
        );

        // A distinct fresh nonce is accepted.
        assert_eq!(guard.observe_at("n3", now, now), Ok(()));
    }

    #[test]
    fn replay_guard_evicts_aged_nonces() {
        let mut guard = ReplayGuard::new(std::time::Duration::from_secs(30));
        let t0 = 1_000_000u64;
        assert_eq!(guard.observe_at("old", t0, t0), Ok(()));
        assert_eq!(guard.tracked(), 1);

        // Far in the future: the old nonce ages out, so it could be reused.
        let t1 = t0 + 120_000;
        assert_eq!(guard.observe_at("new", t1, t1), Ok(()));
        assert_eq!(guard.tracked(), 1, "aged nonce should have been evicted");
    }

    #[test]
    fn replay_guard_caps_tracked_nonces_and_fails_closed() {
        let mut guard = ReplayGuard::with_max_tracked(std::time::Duration::from_secs(30), 3);
        let now = 1_000_000u64;

        assert_eq!(guard.observe_at("n1", now, now), Ok(()));
        assert_eq!(guard.observe_at("n2", now, now), Ok(()));
        assert_eq!(guard.observe_at("n3", now, now), Ok(()));
        assert_eq!(guard.tracked(), 3);

        // Guard is full: a fresh, unseen nonce is REJECTED (fail closed),
        // not silently accepted untracked.
        assert_eq!(
            guard.observe_at("n4", now, now),
            Err(ReplayError::CapacityExhausted)
        );
        assert_eq!(guard.tracked(), 3, "rejected nonce must not be stored");

        // A replayed nonce still reports Replay, even at capacity.
        assert_eq!(guard.observe_at("n1", now, now), Err(ReplayError::Replay));

        // Once entries age out of the window, capacity frees up again.
        let later = now + 120_000;
        assert_eq!(guard.observe_at("n5", later, later), Ok(()));
        assert_eq!(guard.tracked(), 1);
    }

    #[test]
    fn replay_guard_default_cap_is_max_seen() {
        let guard = ReplayGuard::new(std::time::Duration::from_secs(30));
        assert_eq!(guard.max_tracked, MAX_SEEN);
    }

    #[test]
    fn nonce_is_unique() {
        assert_ne!(fresh_nonce(), fresh_nonce());
    }

    #[test]
    fn incomplete_frame_returns_error() {
        let msg = ControlMessage::ListCapabilities;
        let encoded = encode_message(&msg).unwrap();
        // Truncate
        let result: Result<(ControlMessage, usize), _> =
            decode_message(&encoded[..encoded.len() - 1]);
        assert!(result.is_err());
    }
}
