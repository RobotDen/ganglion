//! ROS 2 topic echo capability for Ganglion.
//!
//! Subscribes to specified ROS 2 topics and captures serialized messages
//! with optional decimation (sampling every Nth message). Designed for
//! remote inspection of topic traffic without requiring a local ROS
//! installation on the operator side.
//!
//! When compiled to a WASM component this uses the `ros-interface` host
//! import (`topic-subscribe`) to receive serialized messages. As a
//! native library the capture and decimation logic is testable without
//! ROS.
//!
//! The design spec designates this as the C++ reference capability
//! (wasi-sdk + wit-bindgen). The Rust crate implements the canonical
//! logic; a C++ example project would demonstrate native-language parity
//! for the ROS 2 community.

use serde::{Deserialize, Serialize};

/// Configuration for a topic echo capture session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EchoConfig {
    /// Topic names to subscribe to.
    pub topics: Vec<String>,
    /// Decimation factor: capture every Nth message. 1 = all messages.
    pub decimation: u32,
    /// Maximum number of messages to capture per topic (0 = unlimited).
    pub max_messages: u32,
}

impl Default for EchoConfig {
    fn default() -> Self {
        Self {
            topics: Vec::new(),
            decimation: 1,
            max_messages: 10,
        }
    }
}

/// A single captured topic message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapturedMessage {
    /// Topic this message was received on.
    pub topic: String,
    /// Sequence number within this capture session.
    pub sequence: u64,
    /// ISO 8601 timestamp of capture.
    pub timestamp: String,
    /// Serialized message data (typically CDR-encoded by ROS 2).
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,
    /// Size of the serialized message in bytes.
    pub size_bytes: usize,
}

/// Result of a topic echo session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EchoReport {
    /// Configuration used for this session.
    pub config: EchoConfig,
    /// Captured messages across all topics.
    pub messages: Vec<CapturedMessage>,
    /// Per-topic statistics.
    pub topic_stats: Vec<TopicStat>,
    /// Total messages received (before decimation).
    pub total_received: u64,
    /// Total messages captured (after decimation).
    pub total_captured: u64,
}

/// Per-topic statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicStat {
    /// Topic name.
    pub topic: String,
    /// Messages received on this topic.
    pub received: u64,
    /// Messages captured (after decimation).
    pub captured: u64,
    /// Total bytes of captured message data.
    pub bytes_captured: u64,
}

/// Serde module for base64-encoded byte arrays in JSON.
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Simple hex encoding for portability (no base64 dep needed)
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        serializer.serialize_str(&hex)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(serde::de::Error::custom))
            .collect()
    }
}

/// Apply decimation to a stream of raw messages for a single topic.
///
/// Returns only every Nth message (where N = `decimation`), up to
/// `max_messages` total. Each captured message gets a sequential number.
pub fn decimate(
    topic: &str,
    raw_messages: &[Vec<u8>],
    decimation: u32,
    max_messages: u32,
    timestamp: &str,
) -> (Vec<CapturedMessage>, TopicStat) {
    let decimation = decimation.max(1);
    let mut captured = Vec::new();
    let mut seq: u64 = 0;

    for (i, data) in raw_messages.iter().enumerate() {
        if !(i as u32).is_multiple_of(decimation) {
            continue;
        }
        if max_messages > 0 && captured.len() >= max_messages as usize {
            break;
        }
        seq += 1;
        captured.push(CapturedMessage {
            topic: topic.to_string(),
            sequence: seq,
            timestamp: timestamp.to_string(),
            size_bytes: data.len(),
            data: data.clone(),
        });
    }

    let bytes_captured: u64 = captured.iter().map(|m| m.size_bytes as u64).sum();
    let stat = TopicStat {
        topic: topic.to_string(),
        received: raw_messages.len() as u64,
        captured: captured.len() as u64,
        bytes_captured,
    };

    (captured, stat)
}

/// Build an echo report from pre-captured data (for testing and native mode).
pub fn build_report(
    config: &EchoConfig,
    topic_results: Vec<(Vec<CapturedMessage>, TopicStat)>,
) -> EchoReport {
    let mut all_messages = Vec::new();
    let mut topic_stats = Vec::new();
    let mut total_received: u64 = 0;
    let mut total_captured: u64 = 0;

    for (messages, stat) in topic_results {
        total_received += stat.received;
        total_captured += stat.captured;
        topic_stats.push(stat);
        all_messages.extend(messages);
    }

    EchoReport {
        config: config.clone(),
        messages: all_messages,
        topic_stats,
        total_received,
        total_captured,
    }
}

/// Format an echo report as human-readable text.
pub fn format_report(report: &EchoReport) -> String {
    let mut out = String::new();
    out.push_str("Topic Echo Report\n");
    out.push_str(&"=".repeat(50));
    out.push('\n');

    out.push_str(&format!(
        "\nDecimation: 1/{}, Max per topic: {}\n",
        report.config.decimation,
        if report.config.max_messages == 0 {
            "unlimited".to_string()
        } else {
            report.config.max_messages.to_string()
        }
    ));

    out.push_str(&format!(
        "Total received: {}, captured: {}\n",
        report.total_received, report.total_captured
    ));

    out.push_str("\n## Per-topic stats\n");
    for stat in &report.topic_stats {
        out.push_str(&format!(
            "  {}: {}/{} messages, {} bytes\n",
            stat.topic, stat.captured, stat.received, stat.bytes_captured
        ));
    }

    out.push_str("\n## Captured messages\n");
    for msg in &report.messages {
        out.push_str(&format!(
            "  [{}] #{} on {} ({} bytes)\n",
            msg.timestamp, msg.sequence, msg.topic, msg.size_bytes
        ));
    }

    out
}

/// Parse CLI arguments into an EchoConfig.
///
/// Expected format: `topic1 [topic2 ...] [--decimation N] [--max N]`
pub fn parse_args(args: &[String]) -> EchoConfig {
    let mut config = EchoConfig::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--decimation" | "-d" => {
                if i + 1 < args.len() {
                    config.decimation = args[i + 1].parse().unwrap_or(1);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--max" | "-n" => {
                if i + 1 < args.len() {
                    config.max_messages = args[i + 1].parse().unwrap_or(10);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            topic => {
                config.topics.push(topic.to_string());
                i += 1;
            }
        }
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimate_no_reduction() {
        let messages: Vec<Vec<u8>> = vec![vec![1, 2], vec![3, 4], vec![5, 6]];
        let (captured, stat) = decimate("/topic", &messages, 1, 0, "2026-04-23T12:00:00Z");
        assert_eq!(captured.len(), 3);
        assert_eq!(stat.received, 3);
        assert_eq!(stat.captured, 3);
    }

    #[test]
    fn decimate_every_second() {
        let messages: Vec<Vec<u8>> = (0..10).map(|i| vec![i]).collect();
        let (captured, stat) = decimate("/cmd_vel", &messages, 2, 0, "2026-04-23T12:00:00Z");
        assert_eq!(captured.len(), 5); // indices 0, 2, 4, 6, 8
        assert_eq!(stat.received, 10);
        assert_eq!(stat.captured, 5);
        assert_eq!(captured[0].data, vec![0]);
        assert_eq!(captured[1].data, vec![2]);
    }

    #[test]
    fn decimate_with_max() {
        let messages: Vec<Vec<u8>> = (0..20).map(|i| vec![i]).collect();
        let (captured, stat) = decimate("/odom", &messages, 1, 5, "2026-04-23T12:00:00Z");
        assert_eq!(captured.len(), 5);
        assert_eq!(stat.received, 20);
        assert_eq!(stat.captured, 5);
    }

    #[test]
    fn decimate_empty_stream() {
        let messages: Vec<Vec<u8>> = Vec::new();
        let (captured, stat) = decimate("/empty", &messages, 1, 10, "2026-04-23T12:00:00Z");
        assert!(captured.is_empty());
        assert_eq!(stat.received, 0);
        assert_eq!(stat.captured, 0);
    }

    #[test]
    fn sequence_numbers_ascending() {
        let messages: Vec<Vec<u8>> = vec![vec![1], vec![2], vec![3]];
        let (captured, _) = decimate("/seq", &messages, 1, 0, "2026-04-23T12:00:00Z");
        let seqs: Vec<u64> = captured.iter().map(|m| m.sequence).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
    }

    #[test]
    fn build_report_aggregates() {
        let config = EchoConfig {
            topics: vec!["/a".into(), "/b".into()],
            decimation: 1,
            max_messages: 10,
        };
        let (msgs_a, stat_a) = decimate("/a", &[vec![1], vec![2]], 1, 10, "2026-04-23T12:00:00Z");
        let (msgs_b, stat_b) = decimate(
            "/b",
            &[vec![3], vec![4], vec![5]],
            1,
            10,
            "2026-04-23T12:00:00Z",
        );

        let report = build_report(&config, vec![(msgs_a, stat_a), (msgs_b, stat_b)]);
        assert_eq!(report.total_received, 5);
        assert_eq!(report.total_captured, 5);
        assert_eq!(report.messages.len(), 5);
        assert_eq!(report.topic_stats.len(), 2);
    }

    #[test]
    fn parse_args_basic() {
        let args: Vec<String> = vec!["/cmd_vel", "/odom", "--decimation", "5", "--max", "20"]
            .into_iter()
            .map(String::from)
            .collect();
        let config = parse_args(&args);
        assert_eq!(config.topics, vec!["/cmd_vel", "/odom"]);
        assert_eq!(config.decimation, 5);
        assert_eq!(config.max_messages, 20);
    }

    #[test]
    fn parse_args_defaults() {
        let args: Vec<String> = vec!["/scan"].into_iter().map(String::from).collect();
        let config = parse_args(&args);
        assert_eq!(config.topics, vec!["/scan"]);
        assert_eq!(config.decimation, 1);
        assert_eq!(config.max_messages, 10);
    }

    #[test]
    fn serialization_roundtrip() {
        let msg = CapturedMessage {
            topic: "/test".into(),
            sequence: 1,
            timestamp: "2026-04-23T12:00:00Z".into(),
            data: vec![0xde, 0xad, 0xbe, 0xef],
            size_bytes: 4,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let loaded: CapturedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, loaded);
    }

    #[test]
    fn format_report_contains_sections() {
        let config = EchoConfig {
            topics: vec!["/scan".into()],
            decimation: 2,
            max_messages: 5,
        };
        let (msgs, stat) = decimate(
            "/scan",
            &[vec![1], vec![2], vec![3], vec![4]],
            2,
            5,
            "2026-04-23T12:00:00Z",
        );
        let report = build_report(&config, vec![(msgs, stat)]);
        let text = format_report(&report);
        assert!(text.contains("Topic Echo Report"));
        assert!(text.contains("1/2")); // decimation
        assert!(text.contains("/scan"));
        assert!(text.contains("Per-topic stats"));
        assert!(text.contains("Captured messages"));
    }

    #[test]
    fn bytes_captured_tracking() {
        let messages: Vec<Vec<u8>> = vec![vec![1, 2, 3], vec![4, 5]];
        let (_, stat) = decimate("/sized", &messages, 1, 0, "2026-04-23T12:00:00Z");
        assert_eq!(stat.bytes_captured, 5); // 3 + 2
    }
}

/// WASM component entry point — bridges the `ganglion-capability` world's
/// `run` export to this crate's canonical logic (wasm32 builds only; see
/// `component.rs`). Native builds and tests are unaffected.
#[cfg(target_arch = "wasm32")]
mod component;
