//! Log normalization capability for Ganglion.
//!
//! Converts varied log formats (systemd/journald, ROS 2, syslog, and
//! free-form text) into a unified structured schema suitable for
//! fleet-wide analysis and aggregation.
//!
//! When compiled to a WASM component this uses the `logs-stream` host
//! interface to fetch raw log lines. As a native library the parsing
//! and normalization logic is testable without host access.
//!
//! The design spec designates this capability as the Python reference
//! (componentize-py). The Rust crate implements the canonical logic;
//! the `examples/python/` project in the repository root demonstrates
//! the same algorithm authored in Python for the multi-language pathway.

use serde::{Deserialize, Serialize};

/// A single normalized log entry produced from any supported format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedEntry {
    /// ISO 8601 timestamp, if parseable from the source line.
    pub timestamp: Option<String>,
    /// Severity level mapped to a common enum.
    pub severity: Severity,
    /// Source identifier (unit name, node name, facility, or filename).
    pub source: String,
    /// The log message body with format-specific prefixes stripped.
    pub message: String,
    /// Which parser matched this line.
    pub format: LogFormat,
}

/// Common severity levels across all log formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Unknown,
}

/// Detected source format of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogFormat {
    /// systemd journal / journalctl output.
    Journald,
    /// ROS 2 console log format (`[severity] [timestamp] [node]: msg`).
    Ros2,
    /// BSD/RFC 3164 syslog.
    Syslog,
    /// Unrecognized format — message preserved verbatim.
    Plaintext,
}

/// Normalization report for a batch of log lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationReport {
    /// Total lines processed.
    pub total_lines: usize,
    /// Lines successfully parsed with a known format.
    pub parsed_lines: usize,
    /// Lines that fell through to plaintext.
    pub plaintext_lines: usize,
    /// Breakdown by detected format.
    pub format_counts: FormatCounts,
    /// The normalized entries.
    pub entries: Vec<NormalizedEntry>,
}

/// Per-format line counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FormatCounts {
    pub journald: usize,
    pub ros2: usize,
    pub syslog: usize,
    pub plaintext: usize,
}

/// Parse a severity keyword into the common enum.
fn parse_severity(s: &str) -> Severity {
    match s.to_ascii_lowercase().as_str() {
        "debug" | "dbg" | "7" => Severity::Debug,
        "info" | "information" | "notice" | "6" | "5" => Severity::Info,
        "warn" | "warning" | "4" => Severity::Warn,
        "error" | "err" | "3" => Severity::Error,
        "fatal" | "critical" | "crit" | "alert" | "emerg" | "panic" | "0" | "1" | "2" => {
            Severity::Fatal
        }
        _ => Severity::Unknown,
    }
}

/// Try to parse a line as ROS 2 console output.
///
/// ROS 2 format: `[severity] [timestamp] [node_name]: message`
fn try_parse_ros2(line: &str) -> Option<NormalizedEntry> {
    let line = line.trim();
    if !line.starts_with('[') {
        return None;
    }
    // Extract first bracket pair — severity
    let sev_end = line.find(']')?;
    let sev_str = &line[1..sev_end];

    let rest = line[sev_end + 1..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }

    // Extract second bracket pair — timestamp
    let ts_end = rest.find(']')?;
    let ts_str = &rest[1..ts_end];

    let rest = rest[ts_end + 1..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }

    // Extract third bracket pair — node name
    let node_end = rest.find(']')?;
    let node_str = &rest[1..node_end];

    let rest = rest[node_end + 1..].trim_start();
    let message = rest.strip_prefix(':').unwrap_or(rest).trim_start();

    Some(NormalizedEntry {
        timestamp: Some(ts_str.to_string()),
        severity: parse_severity(sev_str),
        source: node_str.to_string(),
        message: message.to_string(),
        format: LogFormat::Ros2,
    })
}

/// Try to parse a line as journald output.
///
/// Journald format: `Mon DD HH:MM:SS hostname unit[pid]: message`
fn try_parse_journald(line: &str) -> Option<NormalizedEntry> {
    let line = line.trim();
    // Journald lines start with a 3-letter month abbreviation
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let first_word = line.split_whitespace().next()?;
    if !months.contains(&first_word) {
        return None;
    }

    let parts: Vec<&str> = line.splitn(6, ' ').collect();
    if parts.len() < 6 {
        return None;
    }

    // parts: [month, day, time, hostname, unit_pid, message...]
    let timestamp = format!("{} {} {}", parts[0], parts[1], parts[2]);
    // parts[3] is the hostname; parts[4] is the unit[pid]
    let source_field = parts[4].trim_end_matches(':');

    // Strip [pid] from source
    let source = if let Some(bracket) = source_field.find('[') {
        &source_field[..bracket]
    } else {
        source_field
    };

    let message = parts[5];

    Some(NormalizedEntry {
        timestamp: Some(timestamp),
        severity: Severity::Info, // journald text output doesn't include severity inline
        source: source.to_string(),
        message: message.to_string(),
        format: LogFormat::Journald,
    })
}

/// Try to parse a line as BSD syslog (RFC 3164).
///
/// Syslog format: `<priority>Mon DD HH:MM:SS hostname app[pid]: message`
fn try_parse_syslog(line: &str) -> Option<NormalizedEntry> {
    let line = line.trim();
    if !line.starts_with('<') {
        return None;
    }

    let pri_end = line.find('>')?;
    let pri_str = &line[1..pri_end];
    let priority: u8 = pri_str.parse().ok()?;

    // Severity is priority % 8
    let sev_num = priority % 8;
    let severity = parse_severity(&sev_num.to_string());

    // Rest is similar to journald format
    let rest = &line[pri_end + 1..];
    let parts: Vec<&str> = rest.splitn(5, ' ').collect();
    if parts.len() < 5 {
        return None;
    }

    let timestamp = format!("{} {} {}", parts[0], parts[1], parts[2]);
    let source_field = parts[3];
    let message = parts[4];

    // Strip trailing colon from source
    let source = source_field.trim_end_matches(':');

    Some(NormalizedEntry {
        timestamp: Some(timestamp),
        severity,
        source: source.to_string(),
        message: message.to_string(),
        format: LogFormat::Syslog,
    })
}

/// Parse a single line as plaintext fallback.
fn parse_plaintext(line: &str) -> NormalizedEntry {
    NormalizedEntry {
        timestamp: None,
        severity: Severity::Unknown,
        source: String::new(),
        message: line.trim().to_string(),
        format: LogFormat::Plaintext,
    }
}

/// Normalize a single log line by trying each parser in priority order.
pub fn normalize_line(line: &str) -> NormalizedEntry {
    if line.trim().is_empty() {
        return parse_plaintext(line);
    }

    try_parse_ros2(line)
        .or_else(|| try_parse_syslog(line))
        .or_else(|| try_parse_journald(line))
        .unwrap_or_else(|| parse_plaintext(line))
}

/// Normalize a batch of log lines and produce a report.
pub fn normalize_batch(lines: &[&str]) -> NormalizationReport {
    let mut entries = Vec::with_capacity(lines.len());
    let mut counts = FormatCounts::default();

    for line in lines {
        let entry = normalize_line(line);
        match entry.format {
            LogFormat::Journald => counts.journald += 1,
            LogFormat::Ros2 => counts.ros2 += 1,
            LogFormat::Syslog => counts.syslog += 1,
            LogFormat::Plaintext => counts.plaintext += 1,
        }
        entries.push(entry);
    }

    let total = entries.len();
    let plaintext = counts.plaintext;

    NormalizationReport {
        total_lines: total,
        parsed_lines: total - plaintext,
        plaintext_lines: plaintext,
        format_counts: counts,
        entries,
    }
}

/// Format a normalization report as human-readable text.
pub fn format_report(report: &NormalizationReport) -> String {
    let mut out = String::new();
    out.push_str("Log Normalization Report\n");
    out.push_str(&"=".repeat(50));
    out.push('\n');

    out.push_str(&format!(
        "\nProcessed: {} lines ({} parsed, {} plaintext)\n",
        report.total_lines, report.parsed_lines, report.plaintext_lines
    ));
    out.push_str(&format!(
        "Formats:   journald={}, ros2={}, syslog={}, plaintext={}\n",
        report.format_counts.journald,
        report.format_counts.ros2,
        report.format_counts.syslog,
        report.format_counts.plaintext,
    ));

    out.push('\n');
    out.push_str(&"─".repeat(50));
    out.push('\n');

    for entry in &report.entries {
        let ts = entry.timestamp.as_deref().unwrap_or("-");
        let sev = format!("{:?}", entry.severity).to_uppercase();
        let src = if entry.source.is_empty() {
            "-"
        } else {
            &entry.source
        };
        out.push_str(&format!("[{sev:<5}] {ts} [{src}] {}\n", entry.message));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ros2_line() {
        let line = "[INFO] [1682345678.123] [/talker]: Hello world";
        let entry = normalize_line(line);
        assert_eq!(entry.format, LogFormat::Ros2);
        assert_eq!(entry.severity, Severity::Info);
        assert_eq!(entry.source, "/talker");
        assert_eq!(entry.message, "Hello world");
        assert_eq!(entry.timestamp.as_deref(), Some("1682345678.123"));
    }

    #[test]
    fn parse_ros2_warn() {
        let line = "[WARN] [1682345679.456] [/controller]: Timeout exceeded";
        let entry = normalize_line(line);
        assert_eq!(entry.format, LogFormat::Ros2);
        assert_eq!(entry.severity, Severity::Warn);
        assert_eq!(entry.source, "/controller");
        assert_eq!(entry.message, "Timeout exceeded");
    }

    #[test]
    fn parse_journald_line() {
        let line = "Apr 23 14:30:15 robot-01 nav2_controller[1234]: Planning path to goal";
        let entry = normalize_line(line);
        assert_eq!(entry.format, LogFormat::Journald);
        assert_eq!(entry.source, "nav2_controller");
        assert_eq!(entry.message, "Planning path to goal");
        assert!(entry.timestamp.is_some());
    }

    #[test]
    fn parse_syslog_line() {
        let line = "<134>Apr 23 14:30:15 robot-01 sshd[5678]: Accepted publickey for user";
        let entry = normalize_line(line);
        assert_eq!(entry.format, LogFormat::Syslog);
        assert_eq!(entry.severity, Severity::Info); // 134 % 8 = 6 = info
        assert_eq!(entry.source, "robot-01");
        assert_eq!(entry.message, "sshd[5678]: Accepted publickey for user");
    }

    #[test]
    fn parse_plaintext_fallback() {
        let line = "some random log output without structure";
        let entry = normalize_line(line);
        assert_eq!(entry.format, LogFormat::Plaintext);
        assert_eq!(entry.severity, Severity::Unknown);
        assert!(entry.timestamp.is_none());
        assert_eq!(entry.message, line);
    }

    #[test]
    fn parse_empty_line() {
        let entry = normalize_line("");
        assert_eq!(entry.format, LogFormat::Plaintext);
        assert!(entry.message.is_empty());
    }

    #[test]
    fn batch_normalize_mixed() {
        let lines = vec![
            "[INFO] [1682345678.123] [/talker]: Hello",
            "Apr 23 14:30:15 robot-01 systemd[1]: Started service",
            "<131>Apr 23 14:30:16 robot-01 kernel: OOM killer invoked",
            "just plain text",
        ];
        let report = normalize_batch(&lines);
        assert_eq!(report.total_lines, 4);
        assert_eq!(report.parsed_lines, 3);
        assert_eq!(report.plaintext_lines, 1);
        assert_eq!(report.format_counts.ros2, 1);
        assert_eq!(report.format_counts.journald, 1);
        assert_eq!(report.format_counts.syslog, 1);
        assert_eq!(report.format_counts.plaintext, 1);
    }

    #[test]
    fn severity_parsing() {
        assert_eq!(parse_severity("DEBUG"), Severity::Debug);
        assert_eq!(parse_severity("info"), Severity::Info);
        assert_eq!(parse_severity("WARN"), Severity::Warn);
        assert_eq!(parse_severity("warning"), Severity::Warn);
        assert_eq!(parse_severity("ERROR"), Severity::Error);
        assert_eq!(parse_severity("err"), Severity::Error);
        assert_eq!(parse_severity("FATAL"), Severity::Fatal);
        assert_eq!(parse_severity("critical"), Severity::Fatal);
        assert_eq!(parse_severity("banana"), Severity::Unknown);
    }

    #[test]
    fn serialization_roundtrip() {
        let entry = NormalizedEntry {
            timestamp: Some("2026-04-23T14:30:15Z".into()),
            severity: Severity::Warn,
            source: "/nav2".into(),
            message: "Path blocked".into(),
            format: LogFormat::Ros2,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let loaded: NormalizedEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, loaded);
    }

    #[test]
    fn format_report_contains_sections() {
        let lines = vec![
            "[ERROR] [1682345678.123] [/motor]: Overcurrent detected",
            "Apr 23 14:30:15 robot-01 systemd[1]: Started service",
        ];
        let report = normalize_batch(&lines);
        let text = format_report(&report);
        assert!(text.contains("Log Normalization Report"));
        assert!(text.contains("2 lines"));
        assert!(text.contains("2 parsed"));
        assert!(text.contains("0 plaintext"));
        assert!(text.contains("Overcurrent detected"));
    }

    #[test]
    fn syslog_severity_extraction() {
        // Priority 11 = facility 1 (user), severity 3 (error)
        let line = "<11>Apr 23 14:30:15 host app: crash";
        let entry = normalize_line(line);
        assert_eq!(entry.format, LogFormat::Syslog);
        assert_eq!(entry.severity, Severity::Error); // 11 % 8 = 3
    }
}

/// WASM component entry point — bridges the `ganglion-capability` world's
/// `run` export to this crate's canonical logic (wasm32 builds only; see
/// `component.rs`). Native builds and tests are unaffected.
#[cfg(target_arch = "wasm32")]
mod component;
