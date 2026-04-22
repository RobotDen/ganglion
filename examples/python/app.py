"""Log normalization capability — Python reference implementation.

This is the Python version of gang-capability-log-normalize, demonstrating
the componentize-py toolchain for Ganglion capability authoring.

Build:
    componentize-py -d ./wit -w ganglion-capability componentize app \
        -o log-normalize.component.wasm

Sign and deploy:
    gang sign log-normalize.component.wasm --name log-normalize --version 0.1.0
    gang deploy robot-42 log-normalize.component.wasm
    gang run robot-42 log-normalize
"""

import json
import re


# Severity levels mapped to a common set
SEVERITY_MAP = {
    "debug": "debug",
    "dbg": "debug",
    "7": "debug",
    "info": "info",
    "information": "info",
    "notice": "info",
    "6": "info",
    "5": "info",
    "warn": "warn",
    "warning": "warn",
    "4": "warn",
    "error": "error",
    "err": "error",
    "3": "error",
    "fatal": "fatal",
    "critical": "fatal",
    "crit": "fatal",
    "alert": "fatal",
    "emerg": "fatal",
    "panic": "fatal",
    "0": "fatal",
    "1": "fatal",
    "2": "fatal",
}

MONTHS = {"Jan", "Feb", "Mar", "Apr", "May", "Jun",
          "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"}

# ROS 2 log pattern: [SEVERITY] [timestamp] [node]: message
ROS2_PATTERN = re.compile(
    r"^\[(\w+)\]\s+\[([^\]]+)\]\s+\[([^\]]+)\]:?\s*(.*)"
)

# Syslog pattern: <priority>Mon DD HH:MM:SS ...
SYSLOG_PATTERN = re.compile(r"^<(\d+)>(.+)")


def parse_severity(s: str) -> str:
    """Map a severity keyword to a common level."""
    return SEVERITY_MAP.get(s.lower(), "unknown")


def try_parse_ros2(line: str) -> dict | None:
    """Try to parse a line as ROS 2 console output."""
    m = ROS2_PATTERN.match(line.strip())
    if not m:
        return None
    return {
        "timestamp": m.group(2),
        "severity": parse_severity(m.group(1)),
        "source": m.group(3),
        "message": m.group(4),
        "format": "ros2",
    }


def try_parse_journald(line: str) -> dict | None:
    """Try to parse a line as journald output."""
    parts = line.strip().split(None, 5)
    if len(parts) < 6:
        return None
    if parts[0] not in MONTHS:
        return None
    timestamp = f"{parts[0]} {parts[1]} {parts[2]}"
    source_field = parts[4].rstrip(":")
    # Strip [pid] from source
    bracket = source_field.find("[")
    source = source_field[:bracket] if bracket >= 0 else source_field
    return {
        "timestamp": timestamp,
        "severity": "info",
        "source": source,
        "message": parts[5],
        "format": "journald",
    }


def try_parse_syslog(line: str) -> dict | None:
    """Try to parse a line as BSD syslog (RFC 3164)."""
    m = SYSLOG_PATTERN.match(line.strip())
    if not m:
        return None
    priority = int(m.group(1))
    severity = parse_severity(str(priority % 8))
    rest = m.group(2)
    parts = rest.split(None, 4)
    if len(parts) < 5:
        return None
    timestamp = f"{parts[0]} {parts[1]} {parts[2]}"
    return {
        "timestamp": timestamp,
        "severity": severity,
        "source": parts[3].rstrip(":"),
        "message": parts[4],
        "format": "syslog",
    }


def normalize_line(line: str) -> dict:
    """Normalize a single log line by trying parsers in priority order."""
    if not line.strip():
        return {
            "timestamp": None,
            "severity": "unknown",
            "source": "",
            "message": "",
            "format": "plaintext",
        }

    for parser in (try_parse_ros2, try_parse_syslog, try_parse_journald):
        result = parser(line)
        if result is not None:
            return result

    return {
        "timestamp": None,
        "severity": "unknown",
        "source": "",
        "message": line.strip(),
        "format": "plaintext",
    }


def normalize_batch(lines: list[str]) -> dict:
    """Normalize a batch of log lines and produce a report."""
    entries = [normalize_line(line) for line in lines]
    counts = {"journald": 0, "ros2": 0, "syslog": 0, "plaintext": 0}
    for entry in entries:
        counts[entry["format"]] = counts.get(entry["format"], 0) + 1

    plaintext = counts["plaintext"]
    return {
        "total_lines": len(entries),
        "parsed_lines": len(entries) - plaintext,
        "plaintext_lines": plaintext,
        "format_counts": counts,
        "entries": entries,
    }


class GanglionCapability:
    """Implements the ganglion-capability world for componentize-py.

    When built as a WASM component, this class is the entry point.
    The class name must match the world name in PascalCase.
    """

    def run(self, args: list[str]) -> bytes:
        """Entry point called by the Ganglion runtime.

        In WASM mode, fetches logs via the logs-stream host interface.
        When called directly, reads from stdin or uses sample data.
        """
        # In a real WASM component, we'd import the host interface:
        # from ganglion_capability.imports import logs_stream
        # sources = logs_stream.list_sources()
        # lines = logs_stream.stream_logs(sources[0].name, "")

        # For the reference example, use sample data
        sample_lines = [
            "[INFO] [1682345678.123] [/talker]: Publishing hello",
            "Apr 23 14:30:15 robot-01 nav2[1234]: Planning path",
            "<134>Apr 23 14:30:16 robot-01 sshd[5678]: Connection accepted",
            "unstructured log output",
        ]

        report = normalize_batch(sample_lines)
        return json.dumps(report, indent=2).encode("utf-8")
