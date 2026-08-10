//! Robot-side live ROS topic streaming (`/ganglion/topics/1.0`).
//!
//! Mirrors the ADR-024 event push shape: the operator opens a dedicated
//! substream, sends one `TopicStreamRequest`, and receives framed
//! `TopicStreamMessage`s until either side closes. Governance is enforced
//! HERE, on the robot: the subscriber must pass the same trust rule as deploy
//! and the event feed, and every requested topic is evaluated by the
//! default-deny policy engine as a read-only `ganglion:ros/interface` pattern
//! — so the per-topic globs and read-only ceiling in `policy.toml` apply to
//! live streaming exactly as they do to deployed capabilities. Each verdict is
//! emitted as a `PolicyDecision` on the event bus, so topic subscriptions are
//! visible in `gang connect` / `gang tui` and the audit surface like any other
//! policy decision.
//!
//! Samples come from `ros2 topic echo <topic>` (the same RMW-agnostic CLI the
//! ROS brokers use), whose YAML document stream is converted to JSON on the
//! robot and shaped (decimation, per-message size cap, rate ceiling) BEFORE it
//! crosses the wire, so a thin uplink never carries data the operator would
//! throw away. The YAML subset converter and the shaper are pure and
//! unit-tested; only the spawn/pump glue touches processes and sockets.

use std::sync::Arc;

use gang_core::topics::{TopicStreamMessage, TopicStreamRequest};
use tracing::{info, warn};

use crate::agent::RobotAgent;

// --- YAML(subset) → JSON -------------------------------------------------------
//
// `ros2 topic echo` emits predictable block-style YAML: nested maps with
// 2-space indentation, block sequences ("- item", possibly at the parent
// key's indent per YAML), plain scalars, and flow lists ("[1, 2, 3]"). This
// converter covers that subset; anything it cannot parse degrades to a JSON
// string rather than an error, so an exotic message never kills the stream.

/// Convert one `ros2 topic echo` YAML document to a JSON value.
pub fn yaml_doc_to_json(doc: &str) -> serde_json::Value {
    let lines: Vec<(usize, &str)> = doc
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| (l.len() - l.trim_start().len(), l.trim_start()))
        .collect();
    if lines.is_empty() {
        return serde_json::Value::Null;
    }
    let (value, _) = parse_block(&lines, 0, lines[0].0);
    value
}

/// Parse a block (mapping or sequence) starting at `i`, whose items sit at
/// `indent`. Returns the value and the index of the first unconsumed line.
fn parse_block(lines: &[(usize, &str)], mut i: usize, indent: usize) -> (serde_json::Value, usize) {
    use serde_json::Value;

    // Sequence?
    if i < lines.len() && lines[i].0 == indent && lines[i].1.starts_with("- ") {
        let mut seq = Vec::new();
        while i < lines.len() && lines[i].0 == indent && lines[i].1.starts_with("- ") {
            let item = &lines[i].1[2..];
            if let Some((k, v)) = split_key_value(item) {
                // "- key: …" starts an inline map item; continuation keys sit
                // at indent + 2 (aligned under the key).
                let mut map = serde_json::Map::new();
                let item_indent = indent + 2;
                if v.is_empty() {
                    let (nested, ni) = parse_nested(lines, i + 1, item_indent);
                    map.insert(k.to_string(), nested);
                    i = ni;
                } else {
                    map.insert(k.to_string(), parse_scalar(v));
                    i += 1;
                }
                while i < lines.len() && lines[i].0 == item_indent {
                    let (mv, ni) = parse_map_entry(lines, i, item_indent);
                    if let Value::Object(o) = mv {
                        map.extend(o);
                    }
                    i = ni;
                }
                seq.push(Value::Object(map));
            } else {
                seq.push(parse_scalar(item));
                i += 1;
            }
        }
        return (Value::Array(seq), i);
    }

    // Mapping.
    let mut map = serde_json::Map::new();
    while i < lines.len() && lines[i].0 == indent && !lines[i].1.starts_with("- ") {
        let (entry, ni) = parse_map_entry(lines, i, indent);
        if let Value::Object(o) = entry {
            map.extend(o);
        }
        i = ni;
    }
    (Value::Object(map), i)
}

/// Parse one `key: …` mapping entry at `indent`, including any nested block.
fn parse_map_entry(lines: &[(usize, &str)], i: usize, indent: usize) -> (serde_json::Value, usize) {
    use serde_json::Value;
    let mut map = serde_json::Map::new();
    let Some((k, v)) = split_key_value(lines[i].1) else {
        // Not a key: value line — degrade to a string entry.
        map.insert(lines[i].1.to_string(), Value::Null);
        return (Value::Object(map), i + 1);
    };
    if !v.is_empty() {
        map.insert(k.to_string(), parse_scalar(v));
        return (Value::Object(map), i + 1);
    }
    let (nested, ni) = parse_nested(lines, i + 1, indent);
    map.insert(k.to_string(), nested);
    (Value::Object(map), ni)
}

/// Parse the value belonging to a `key:` with nothing after the colon: a
/// deeper-indented block, a sequence at parent indent or deeper (both are
/// valid YAML), or null when neither follows.
fn parse_nested(
    lines: &[(usize, &str)],
    i: usize,
    parent_indent: usize,
) -> (serde_json::Value, usize) {
    if i < lines.len() {
        let (ind, content) = lines[i];
        if content.starts_with("- ") && ind >= parent_indent {
            return parse_block(lines, i, ind);
        }
        if ind > parent_indent {
            return parse_block(lines, i, ind);
        }
    }
    (serde_json::Value::Null, i)
}

/// Split `key: value` (value may be empty). Returns `None` for non-map lines.
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(':')?;
    let (k, rest) = line.split_at(idx);
    let v = rest[1..].trim();
    // A colon inside a flow value ("[a: 1]") without a key shape is not a map.
    if k.is_empty() || k.contains('[') || k.contains('{') {
        return None;
    }
    Some((k.trim(), v))
}

/// Parse a YAML scalar into a JSON value (ints, floats, bools, null, flow
/// lists, quoted and plain strings).
fn parse_scalar(s: &str) -> serde_json::Value {
    use serde_json::Value;
    let s = s.trim();
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" => return Value::Bool(true),
        "false" | "False" => return Value::Bool(false),
        _ => {}
    }
    // Quoted string.
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        return Value::String(s[1..s.len() - 1].to_string());
    }
    // Flow sequence: [a, b, c]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        if inner.trim().is_empty() {
            return Value::Array(vec![]);
        }
        return Value::Array(inner.split(',').map(|p| parse_scalar(p.trim())).collect());
    }
    // Numbers.
    if let Ok(n) = s.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = s.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return Value::Number(n);
    }
    Value::String(s.to_string())
}

/// Split a `ros2 topic echo` stdout stream chunk into complete YAML documents
/// using the `---` separator lines (ros2 echo prints one after each message).
/// Returns (complete docs, remaining buffer — the in-progress document,
/// including any final partial line when the chunk did not end in a newline).
pub fn split_yaml_docs(buffer: &str) -> (Vec<String>, String) {
    let mut docs = Vec::new();
    let mut current = String::new();
    let complete_input = buffer.ends_with('\n');
    let lines: Vec<&str> = buffer.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let is_last = i + 1 == lines.len();
        if is_last && !complete_input {
            // Partial final line: keep it (without a newline) for next chunk.
            current.push_str(line);
            break;
        }
        if line.trim() == "---" {
            if !current.trim().is_empty() {
                docs.push(std::mem::take(&mut current));
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    (docs, current)
}

// --- Shaping --------------------------------------------------------------------

/// Robot-side stream shaper: decimation, per-message size cap, rate ceiling.
/// One per topic. Pure (caller supplies the clock) and unit-tested.
#[derive(Debug)]
pub struct Shaper {
    decimation: u64,
    min_interval_ms: Option<u64>,
    max_bytes: Option<u64>,
    seen: u64,
    last_sent_ms: Option<u64>,
}

impl Shaper {
    /// Build a shaper from the request knobs.
    pub fn from_request(req: &TopicStreamRequest) -> Self {
        Self {
            decimation: req.decimation.max(1) as u64,
            min_interval_ms: req.max_rate_hz.and_then(|hz| {
                if hz > 0.0 {
                    Some((1000.0 / hz).round() as u64)
                } else {
                    None
                }
            }),
            max_bytes: req.max_bytes_per_message,
            seen: 0,
            last_sent_ms: None,
        }
    }

    /// Decide whether to forward a sample of `payload_len` bytes observed at
    /// `now_ms` (any monotonic milliseconds clock).
    pub fn admit(&mut self, payload_len: u64, now_ms: u64) -> bool {
        let idx = self.seen;
        self.seen += 1;
        if !idx.is_multiple_of(self.decimation) {
            return false;
        }
        if let Some(cap) = self.max_bytes
            && payload_len > cap
        {
            return false;
        }
        if let (Some(interval), Some(last)) = (self.min_interval_ms, self.last_sent_ms)
            && now_ms.saturating_sub(last) < interval
        {
            return false;
        }
        self.last_sent_ms = Some(now_ms);
        true
    }
}

// --- The per-subscriber stream task ----------------------------------------------

/// Serve one topic-stream subscriber: read the request, authorize, reply with
/// verdicts, then pump shaped samples until the stream closes.
pub async fn run_topic_stream<S>(
    agent: Arc<RobotAgent>,
    subscriber: gang_core::identity::PeerId,
    mut stream: S,
) where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin + Send,
{
    use gang_core::message::{decode_message, encode_message};
    use gang_libp2p::framed::{read_frame, write_frame};

    // One request frame.
    let req: TopicStreamRequest = match read_frame(&mut stream).await {
        Ok(Some(frame)) => match decode_message(&frame) {
            Ok((req, _)) => req,
            Err(e) => {
                warn!(peer = %subscriber, "topic stream: bad request frame: {e}");
                return;
            }
        },
        _ => {
            warn!(peer = %subscriber, "topic stream: no request frame");
            return;
        }
    };

    // Trust + per-topic policy. An untrusted subscriber gets a silent close
    // (same refusal shape as the event feed).
    let verdicts = match agent.authorize_topic_stream(&subscriber, &req.topics) {
        Ok(v) => v,
        Err(e) => {
            warn!(peer = %subscriber, "topic stream refused: {e}");
            return;
        }
    };

    let accepted: Vec<String> = verdicts
        .iter()
        .filter(|v| v.allowed)
        .map(|v| v.topic.clone())
        .collect();

    let reply = TopicStreamMessage::Accepted { verdicts };
    let Ok(frame) = encode_message(&reply) else {
        return;
    };
    if write_frame(&mut stream, &frame).await.is_err() {
        return;
    }
    if accepted.is_empty() {
        return; // verdicts delivered; nothing to stream
    }

    info!(peer = %subscriber, topics = ?accepted, "topic stream: streaming");

    // One echo pump per accepted topic, multiplexed into a single writer.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TopicStreamMessage>(64);
    let mut pumps = Vec::new();
    for topic in accepted {
        let tx = tx.clone();
        let shaper = Shaper::from_request(&req);
        pumps.push(tokio::spawn(pump_topic(topic, shaper, tx)));
    }
    drop(tx);

    while let Some(msg) = rx.recv().await {
        let Ok(frame) = encode_message(&msg) else {
            continue;
        };
        if write_frame(&mut stream, &frame).await.is_err() {
            break; // subscriber went away
        }
    }
    for p in &pumps {
        p.abort();
    }
}

/// Spawn `ros2 topic echo <topic>` and pump shaped samples into `tx` until the
/// process exits or the receiver closes.
async fn pump_topic(
    topic: String,
    mut shaper: Shaper,
    tx: tokio::sync::mpsc::Sender<TopicStreamMessage>,
) {
    use tokio::io::AsyncReadExt;

    let child = tokio::process::Command::new("ros2")
        .args(["topic", "echo", &topic])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = tx
                .send(TopicStreamMessage::TopicError {
                    topic,
                    message: format!("could not start ros2 topic echo: {e}"),
                })
                .await;
            return;
        }
    };
    let Some(mut stdout) = child.stdout.take() else {
        return;
    };

    let started = std::time::Instant::now();
    let mut buffer = String::new();
    let mut chunk = [0u8; 8192];
    let mut seq: u64 = 0;

    loop {
        let n = match stdout.read(&mut chunk).await {
            Ok(0) | Err(_) => break, // echo exited
            Ok(n) => n,
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk[..n]));
        let (docs, rest) = split_yaml_docs(&buffer);
        buffer = rest;
        for doc in docs {
            let json = yaml_doc_to_json(&doc).to_string();
            let now_ms = started.elapsed().as_millis() as u64;
            if !shaper.admit(json.len() as u64, now_ms) {
                continue;
            }
            let msg = TopicStreamMessage::Sample {
                topic: topic.clone(),
                seq,
                ts: chrono::Utc::now(),
                json,
            };
            seq += 1;
            if tx.send(msg).await.is_err() {
                return; // subscriber gone
            }
        }
    }
    let _ = tx
        .send(TopicStreamMessage::TopicError {
            topic,
            message: "ros2 topic echo exited".into(),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_flat_scalar_doc() {
        let v = yaml_doc_to_json("data: hello world\ncount: 42\nok: true\n");
        assert_eq!(v, json!({"data": "hello world", "count": 42, "ok": true}));
    }

    #[test]
    fn converts_nested_header_like_ros2_echo() {
        let doc = "\
header:
  stamp:
    sec: 100
    nanosec: 500000
  frame_id: base_link
pose:
  position:
    x: 1.5
    y: -2.0
    z: 0.0
";
        let v = yaml_doc_to_json(doc);
        assert_eq!(v["header"]["stamp"]["sec"], json!(100));
        assert_eq!(v["header"]["frame_id"], json!("base_link"));
        assert_eq!(v["pose"]["position"]["x"], json!(1.5));
        assert_eq!(v["pose"]["position"]["y"], json!(-2.0));
    }

    #[test]
    fn converts_block_sequences_at_parent_indent() {
        // ros2 echo emits list items at the parent key's indentation.
        let doc = "\
name:
- joint1
- joint2
position:
- 0.5
- 1.25
";
        let v = yaml_doc_to_json(doc);
        assert_eq!(v["name"], json!(["joint1", "joint2"]));
        assert_eq!(v["position"], json!([0.5, 1.25]));
    }

    #[test]
    fn converts_sequence_of_maps() {
        let doc = "\
transforms:
- header:
    frame_id: odom
  child_frame_id: base_link
";
        let v = yaml_doc_to_json(doc);
        assert_eq!(v["transforms"][0]["child_frame_id"], json!("base_link"));
        assert_eq!(v["transforms"][0]["header"]["frame_id"], json!("odom"));
    }

    #[test]
    fn converts_flow_lists_quotes_and_nulls() {
        let v = yaml_doc_to_json("covariance: [0.1, 0.2, 0.3]\nlabel: 'quoted'\nnothing: null\n");
        assert_eq!(v["covariance"], json!([0.1, 0.2, 0.3]));
        assert_eq!(v["label"], json!("quoted"));
        assert_eq!(v["nothing"], json!(null));
    }

    #[test]
    fn splits_docs_on_separators_and_keeps_partial_tail() {
        let (docs, rest) = split_yaml_docs("data: 1\n---\ndata: 2\n---\ndata: 3");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0], "data: 1\n");
        assert_eq!(docs[1], "data: 2\n");
        assert_eq!(rest, "data: 3"); // partial line stays buffered
    }

    #[test]
    fn shaper_decimates() {
        let req = TopicStreamRequest {
            topics: vec![],
            decimation: 3,
            max_rate_hz: None,
            max_bytes_per_message: None,
        };
        let mut s = Shaper::from_request(&req);
        let admitted: Vec<bool> = (0..6).map(|i| s.admit(10, i)).collect();
        assert_eq!(admitted, vec![true, false, false, true, false, false]);
    }

    #[test]
    fn shaper_enforces_size_cap_and_rate() {
        let req = TopicStreamRequest {
            topics: vec![],
            decimation: 1,
            max_rate_hz: Some(2.0), // 500 ms interval
            max_bytes_per_message: Some(100),
        };
        let mut s = Shaper::from_request(&req);
        assert!(s.admit(50, 0)); // first passes
        assert!(!s.admit(50, 100)); // too soon
        assert!(s.admit(50, 600)); // past the interval
        assert!(!s.admit(500, 1200)); // over size cap
    }
}
