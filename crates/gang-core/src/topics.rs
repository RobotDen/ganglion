//! The robot→operator live ROS topic streaming wire model.
//!
//! Carried on the dedicated `/ganglion/topics/1.0` substream
//! ([`crate::protocol::PROTOCOL_TOPICS`]), framed with the same
//! length-prefixed CBOR codec as every other Ganglion protocol. The flow is:
//! the operator opens the substream and sends exactly one
//! `TopicStreamRequest`; the robot authenticates the subscriber with the
//! same trust rule as deploy and the event feed, evaluates the requested
//! topics against the **default-deny policy engine** (each topic becomes a
//! read-only `ganglion:ros/interface` pattern, so per-topic globs and the
//! read-only ceiling in the operator's `policy.toml` apply verbatim), replies
//! with one `TopicStreamMessage::Accepted` naming the per-topic verdicts,
//! and then pushes `TopicStreamMessage::Sample`s live until either side
//! closes the stream.
//!
//! Samples carry the message as a JSON string (converted robot-side from
//! `ros2 topic echo` output) so any consumer — the Foxglove bridge, `--json`
//! tooling, an MCP agent — can use them without a ROS dependency. Stream
//! shaping (decimation, per-message size cap, rate ceiling) is applied on the
//! ROBOT side from the request's knobs, so a thin uplink is never saturated
//! by data the operator would only throw away.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The single request frame an operator sends after opening the topic stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicStreamRequest {
    /// ROS topic names to stream (exact names, e.g. `/rosout`).
    pub topics: Vec<String>,
    /// Forward every Nth message per topic (values < 1 mean 1 = every message).
    #[serde(default = "default_decimation")]
    pub decimation: u32,
    /// Ceiling on forwarded messages per second per topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rate_hz: Option<f64>,
    /// Skip any single sample whose JSON payload exceeds this many bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes_per_message: Option<u64>,
}

fn default_decimation() -> u32 {
    1
}

impl TopicStreamRequest {
    /// A request for `topics` with no shaping (every message, no caps).
    pub fn full(topics: Vec<String>) -> Self {
        Self {
            topics,
            decimation: 1,
            max_rate_hz: None,
            max_bytes_per_message: None,
        }
    }
}

/// Per-topic verdict in the [`TopicStreamMessage::Accepted`] reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicVerdict {
    /// The topic as requested.
    pub topic: String,
    /// Whether the policy engine permitted streaming it.
    pub allowed: bool,
    /// Rationale (policy reason on deny; empty on allow).
    #[serde(default)]
    pub reason: String,
}

/// A frame on the topic stream, robot → operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TopicStreamMessage {
    /// First frame after the request: the per-topic policy verdicts. A request
    /// where every topic was denied still receives this frame (then EOF), so
    /// the operator can distinguish "denied by policy" from a transport error.
    Accepted {
        /// One verdict per requested topic, in request order.
        verdicts: Vec<TopicVerdict>,
    },
    /// One live sample from a permitted topic.
    Sample {
        /// The topic this sample belongs to.
        topic: String,
        /// Per-topic monotonic sequence (post-shaping; gaps are expected under
        /// decimation and rate ceilings).
        seq: u64,
        /// Robot-side receive time.
        ts: DateTime<Utc>,
        /// The message rendered as a JSON document.
        json: String,
    },
    /// A per-topic failure (e.g. `ros2 topic echo` could not start or died).
    /// The stream continues for other topics.
    TopicError {
        /// The affected topic.
        topic: String,
        /// What went wrong (never secret material).
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{decode_message, encode_message};

    #[test]
    fn request_roundtrips_through_codec() {
        let req = TopicStreamRequest {
            topics: vec!["/rosout".into(), "/tf".into()],
            decimation: 5,
            max_rate_hz: Some(2.0),
            max_bytes_per_message: Some(256 * 1024),
        };
        let bytes = encode_message(&req).unwrap();
        let (back, _) = decode_message::<TopicStreamRequest>(&bytes).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn request_defaults_apply_on_sparse_input() {
        // A request serialized without the optional knobs decodes with
        // decimation 1 and no caps.
        let req = TopicStreamRequest::full(vec!["/rosout".into()]);
        let bytes = encode_message(&req).unwrap();
        let (back, _) = decode_message::<TopicStreamRequest>(&bytes).unwrap();
        assert_eq!(back.decimation, 1);
        assert_eq!(back.max_rate_hz, None);
        assert_eq!(back.max_bytes_per_message, None);
    }

    #[test]
    fn messages_roundtrip_through_codec() {
        let msgs = vec![
            TopicStreamMessage::Accepted {
                verdicts: vec![
                    TopicVerdict {
                        topic: "/rosout".into(),
                        allowed: true,
                        reason: String::new(),
                    },
                    TopicVerdict {
                        topic: "/secret".into(),
                        allowed: false,
                        reason: "pattern exceeds policy".into(),
                    },
                ],
            },
            TopicStreamMessage::Sample {
                topic: "/rosout".into(),
                seq: 7,
                ts: Utc::now(),
                json: r#"{"msg":"hello"}"#.into(),
            },
            TopicStreamMessage::TopicError {
                topic: "/tf".into(),
                message: "ros2 not found".into(),
            },
        ];
        for m in msgs {
            let bytes = encode_message(&m).unwrap();
            let (back, _) = decode_message::<TopicStreamMessage>(&bytes).unwrap();
            assert_eq!(back, m);
        }
    }
}
