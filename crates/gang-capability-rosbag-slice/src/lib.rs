//! Rosbag slicing capability for Ganglion.
//!
//! Captures a time-bounded slice of ROS 2 bag data, optionally filtered
//! by topic, and publishes the result as a content-addressed artifact.
//! The operator receives a CID that can be fetched with `gang fetch`.
//!
//! When compiled to a WASM component this uses `ros-interface` (to
//! subscribe to topics), `process-spawn` (to invoke `ros2 bag record`),
//! `fs-bounded` (to read the resulting bag files), and
//! `artifacts-publish` (to publish the slice content-addressed).
//!
//! As a native library the slicing configuration, topic filtering,
//! time-window logic, and bag metadata are testable without ROS.
//!
//! ## Usage
//!
//! ```text
//! gang run robot-42 rosbag-slice --start=-60s --end=now --topics=/odom,/scan,/cmd_vel
//! ```

use serde::{Deserialize, Serialize};

/// Configuration for a rosbag slice operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SliceConfig {
    /// Topics to capture. Empty means all topics.
    pub topics: Vec<String>,
    /// Start time as relative offset (e.g., "-60s") or ISO 8601 timestamp.
    pub start: String,
    /// End time as relative offset or ISO 8601 timestamp.
    pub end: String,
    /// Maximum size of the slice in megabytes (0 = unlimited).
    pub max_size_mb: u64,
    /// Output format for the bag data.
    pub format: BagFormat,
}

/// Supported bag output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BagFormat {
    /// SQLite-based rosbag2 format (default).
    Sqlite3,
    /// MCAP format.
    Mcap,
}

impl Default for SliceConfig {
    fn default() -> Self {
        Self {
            topics: Vec::new(),
            start: "-60s".to_string(),
            end: "now".to_string(),
            max_size_mb: 0,
            format: BagFormat::Sqlite3,
        }
    }
}

/// Metadata about a single topic captured in the slice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicMetadata {
    /// Topic name.
    pub name: String,
    /// ROS 2 message type (e.g., "sensor_msgs/msg/LaserScan").
    pub message_type: String,
    /// Number of messages captured for this topic.
    pub message_count: u64,
    /// Serialized size in bytes for this topic's messages.
    pub size_bytes: u64,
}

/// Result of a rosbag slice operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceResult {
    /// The CID of the published artifact (set after artifact publication).
    pub cid: Option<String>,
    /// Configuration used for this slice.
    pub config: SliceConfig,
    /// ISO 8601 timestamp when capture started.
    pub capture_start: String,
    /// ISO 8601 timestamp when capture ended.
    pub capture_end: String,
    /// Duration of the captured window in seconds.
    pub duration_secs: f64,
    /// Per-topic metadata.
    pub topics: Vec<TopicMetadata>,
    /// Total number of messages captured.
    pub total_messages: u64,
    /// Total size of the bag in bytes.
    pub total_size_bytes: u64,
    /// Output format used.
    pub format: BagFormat,
}

/// Time window specification parsed from CLI arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeWindow {
    /// Offset from reference time in seconds (negative = past).
    pub start_offset_secs: f64,
    /// Offset from reference time in seconds (0 = now).
    pub end_offset_secs: f64,
}

/// Parse a relative time string (e.g., "-60s", "-5m", "now") into seconds offset.
pub fn parse_relative_time(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s == "now" {
        return Ok(0.0);
    }

    let negative = s.starts_with('-');
    let s = s.trim_start_matches('-');

    // Try parsing with unit suffix
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60.0)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600.0)
    } else {
        // Assume seconds
        (s, 1.0)
    };

    let value: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid time value: {num_str}"))?;

    let result = value * multiplier;
    Ok(if negative { -result } else { result })
}

/// Parse start/end arguments into a time window.
pub fn parse_time_window(start: &str, end: &str) -> Result<TimeWindow, String> {
    let start_offset = parse_relative_time(start)?;
    let end_offset = parse_relative_time(end)?;

    if start_offset >= end_offset {
        return Err(format!(
            "start ({start}) must be before end ({end}): {start_offset}s >= {end_offset}s"
        ));
    }

    Ok(TimeWindow {
        start_offset_secs: start_offset,
        end_offset_secs: end_offset,
    })
}

/// Build the `ros2 bag record` command arguments for a slice operation.
pub fn build_record_command(config: &SliceConfig, output_dir: &str) -> Vec<String> {
    let mut args = vec![
        "bag".to_string(),
        "record".to_string(),
        "--output".to_string(),
        output_dir.to_string(),
    ];

    // Storage format
    match config.format {
        BagFormat::Sqlite3 => {
            args.push("--storage".to_string());
            args.push("sqlite3".to_string());
        }
        BagFormat::Mcap => {
            args.push("--storage".to_string());
            args.push("mcap".to_string());
        }
    }

    // Max bag size
    if config.max_size_mb > 0 {
        args.push("--max-bag-size".to_string());
        args.push(format!("{}", config.max_size_mb * 1024 * 1024));
    }

    // Topics or all
    if config.topics.is_empty() {
        args.push("--all".to_string());
    } else {
        for topic in &config.topics {
            args.push(topic.clone());
        }
    }

    args
}

/// Filter a list of available topics against the requested topic patterns.
///
/// Supports exact matches and simple glob patterns (trailing `*`).
pub fn filter_topics(available: &[String], requested: &[String]) -> Vec<String> {
    if requested.is_empty() {
        return available.to_vec();
    }

    available
        .iter()
        .filter(|topic| {
            requested.iter().any(|pattern| {
                if let Some(prefix) = pattern.strip_suffix('*') {
                    topic.starts_with(prefix)
                } else {
                    *topic == pattern
                }
            })
        })
        .cloned()
        .collect()
}

/// Build a slice result from captured metadata (for testing and native mode).
pub fn build_result(
    config: &SliceConfig,
    topics: Vec<TopicMetadata>,
    capture_start: &str,
    capture_end: &str,
    duration_secs: f64,
) -> SliceResult {
    let total_messages: u64 = topics.iter().map(|t| t.message_count).sum();
    let total_size_bytes: u64 = topics.iter().map(|t| t.size_bytes).sum();

    SliceResult {
        cid: None,
        config: config.clone(),
        capture_start: capture_start.to_string(),
        capture_end: capture_end.to_string(),
        duration_secs,
        topics,
        total_messages,
        total_size_bytes,
        format: config.format,
    }
}

/// Parse CLI arguments into a SliceConfig.
///
/// Expected: `[--start TIME] [--end TIME] [--format FORMAT] [--max-size MB] [--topics T1,T2,...]`
pub fn parse_args(args: &[String]) -> Result<SliceConfig, String> {
    let mut config = SliceConfig::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--start" => {
                if i + 1 >= args.len() {
                    return Err("--start requires a value".into());
                }
                config.start = args[i + 1].clone();
                i += 2;
            }
            "--end" => {
                if i + 1 >= args.len() {
                    return Err("--end requires a value".into());
                }
                config.end = args[i + 1].clone();
                i += 2;
            }
            "--topics" => {
                if i + 1 >= args.len() {
                    return Err("--topics requires a value".into());
                }
                config.topics = args[i + 1]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                i += 2;
            }
            "--format" => {
                if i + 1 >= args.len() {
                    return Err("--format requires a value".into());
                }
                config.format = match args[i + 1].as_str() {
                    "sqlite3" => BagFormat::Sqlite3,
                    "mcap" => BagFormat::Mcap,
                    other => {
                        return Err(format!(
                            "unknown format: {other} (expected sqlite3 or mcap)"
                        ));
                    }
                };
                i += 2;
            }
            "--max-size" => {
                if i + 1 >= args.len() {
                    return Err("--max-size requires a value".into());
                }
                config.max_size_mb = args[i + 1]
                    .parse()
                    .map_err(|_| format!("invalid max-size: {}", args[i + 1]))?;
                i += 2;
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
    }

    // Validate the time window
    parse_time_window(&config.start, &config.end)?;

    Ok(config)
}

/// Format a slice result as human-readable text.
pub fn format_report(result: &SliceResult) -> String {
    let mut out = String::new();
    out.push_str("Rosbag Slice Report\n");
    out.push_str(&"=".repeat(50));
    out.push('\n');

    out.push_str(&format!(
        "\nCapture window: {} to {}\n",
        result.capture_start, result.capture_end
    ));
    out.push_str(&format!("Duration: {:.1}s\n", result.duration_secs));
    out.push_str(&format!("Format: {:?}\n", result.format));
    out.push_str(&format!(
        "Total: {} messages, {}\n",
        result.total_messages,
        format_size(result.total_size_bytes)
    ));

    if let Some(cid) = &result.cid {
        out.push_str(&format!("CID: {cid}\n"));
    }

    out.push_str("\n## Topics\n");
    if result.topics.is_empty() {
        out.push_str("  (no topics captured)\n");
    } else {
        for t in &result.topics {
            out.push_str(&format!(
                "  {} [{}]: {} messages, {}\n",
                t.name,
                t.message_type,
                t.message_count,
                format_size(t.size_bytes)
            ));
        }
    }

    out
}

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_relative_seconds() {
        assert_eq!(parse_relative_time("-60s").unwrap(), -60.0);
        assert_eq!(parse_relative_time("30s").unwrap(), 30.0);
        assert_eq!(parse_relative_time("now").unwrap(), 0.0);
    }

    #[test]
    fn parse_relative_minutes() {
        assert_eq!(parse_relative_time("-5m").unwrap(), -300.0);
        assert_eq!(parse_relative_time("2m").unwrap(), 120.0);
    }

    #[test]
    fn parse_relative_hours() {
        assert_eq!(parse_relative_time("-1h").unwrap(), -3600.0);
    }

    #[test]
    fn parse_time_window_valid() {
        let w = parse_time_window("-60s", "now").unwrap();
        assert_eq!(w.start_offset_secs, -60.0);
        assert_eq!(w.end_offset_secs, 0.0);
    }

    #[test]
    fn parse_time_window_invalid_order() {
        let result = parse_time_window("now", "-60s");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be before"));
    }

    #[test]
    fn build_record_command_all_topics() {
        let config = SliceConfig::default();
        let args = build_record_command(&config, "/tmp/slice");
        assert!(args.contains(&"--all".to_string()));
        assert!(args.contains(&"/tmp/slice".to_string()));
        assert!(args.contains(&"sqlite3".to_string()));
    }

    #[test]
    fn build_record_command_specific_topics() {
        let config = SliceConfig {
            topics: vec!["/odom".into(), "/scan".into()],
            format: BagFormat::Mcap,
            ..Default::default()
        };
        let args = build_record_command(&config, "/tmp/slice");
        assert!(args.contains(&"/odom".to_string()));
        assert!(args.contains(&"/scan".to_string()));
        assert!(args.contains(&"mcap".to_string()));
        assert!(!args.contains(&"--all".to_string()));
    }

    #[test]
    fn build_record_command_max_size() {
        let config = SliceConfig {
            max_size_mb: 100,
            ..Default::default()
        };
        let args = build_record_command(&config, "/tmp/slice");
        assert!(args.contains(&"--max-bag-size".to_string()));
        let size_idx = args.iter().position(|a| a == "--max-bag-size").unwrap();
        assert_eq!(args[size_idx + 1], "104857600"); // 100 MB in bytes
    }

    #[test]
    fn filter_topics_exact() {
        let available = vec![
            "/odom".into(),
            "/scan".into(),
            "/cmd_vel".into(),
            "/tf".into(),
        ];
        let requested = vec!["/odom".into(), "/scan".into()];
        let result = filter_topics(&available, &requested);
        assert_eq!(result, vec!["/odom", "/scan"]);
    }

    #[test]
    fn filter_topics_glob() {
        let available = vec![
            "/camera/rgb".into(),
            "/camera/depth".into(),
            "/lidar/scan".into(),
        ];
        let requested = vec!["/camera/*".into()];
        let result = filter_topics(&available, &requested);
        assert_eq!(result, vec!["/camera/rgb", "/camera/depth"]);
    }

    #[test]
    fn filter_topics_empty_captures_all() {
        let available = vec!["/a".into(), "/b".into()];
        let result = filter_topics(&available, &[]);
        assert_eq!(result, vec!["/a", "/b"]);
    }

    #[test]
    fn build_result_aggregates() {
        let config = SliceConfig::default();
        let topics = vec![
            TopicMetadata {
                name: "/odom".into(),
                message_type: "nav_msgs/msg/Odometry".into(),
                message_count: 600,
                size_bytes: 48000,
            },
            TopicMetadata {
                name: "/scan".into(),
                message_type: "sensor_msgs/msg/LaserScan".into(),
                message_count: 60,
                size_bytes: 720000,
            },
        ];
        let result = build_result(
            &config,
            topics,
            "2026-04-23T12:00:00Z",
            "2026-04-23T12:01:00Z",
            60.0,
        );
        assert_eq!(result.total_messages, 660);
        assert_eq!(result.total_size_bytes, 768000);
        assert_eq!(result.topics.len(), 2);
        assert!(result.cid.is_none());
    }

    #[test]
    fn parse_args_full() {
        let args: Vec<String> = vec![
            "--start",
            "-5m",
            "--end",
            "now",
            "--topics",
            "/odom,/scan",
            "--format",
            "mcap",
            "--max-size",
            "50",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let config = parse_args(&args).unwrap();
        assert_eq!(config.start, "-5m");
        assert_eq!(config.end, "now");
        assert_eq!(config.topics, vec!["/odom", "/scan"]);
        assert_eq!(config.format, BagFormat::Mcap);
        assert_eq!(config.max_size_mb, 50);
    }

    #[test]
    fn parse_args_defaults() {
        let config = parse_args(&[]).unwrap();
        assert_eq!(config.start, "-60s");
        assert_eq!(config.end, "now");
        assert!(config.topics.is_empty());
        assert_eq!(config.format, BagFormat::Sqlite3);
    }

    #[test]
    fn parse_args_invalid_time_window() {
        let args: Vec<String> = vec!["--start", "now", "--end", "-60s"]
            .into_iter()
            .map(String::from)
            .collect();
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn format_report_contains_sections() {
        let config = SliceConfig {
            topics: vec!["/odom".into()],
            ..Default::default()
        };
        let topics = vec![TopicMetadata {
            name: "/odom".into(),
            message_type: "nav_msgs/msg/Odometry".into(),
            message_count: 100,
            size_bytes: 8000,
        }];
        let mut result = build_result(
            &config,
            topics,
            "2026-04-23T12:00:00Z",
            "2026-04-23T12:01:00Z",
            60.0,
        );
        result.cid = Some("bafyabc123".into());
        let text = format_report(&result);
        assert!(text.contains("Rosbag Slice Report"));
        assert!(text.contains("60.0s"));
        assert!(text.contains("/odom"));
        assert!(text.contains("nav_msgs/msg/Odometry"));
        assert!(text.contains("bafyabc123"));
    }

    #[test]
    fn format_size_units() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00 GB");
    }

    #[test]
    fn serialization_roundtrip() {
        let config = SliceConfig {
            topics: vec!["/odom".into()],
            start: "-30s".into(),
            end: "now".into(),
            max_size_mb: 100,
            format: BagFormat::Mcap,
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: SliceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn default_config() {
        let config = SliceConfig::default();
        assert!(config.topics.is_empty());
        assert_eq!(config.start, "-60s");
        assert_eq!(config.end, "now");
        assert_eq!(config.max_size_mb, 0);
        assert_eq!(config.format, BagFormat::Sqlite3);
    }
}

/// WASM component entry point — bridges the `ganglion-capability` world's
/// `run` export to this crate's canonical logic (wasm32 builds only; see
/// `component.rs`). Native builds and tests are unaffected.
#[cfg(target_arch = "wasm32")]
mod component;
