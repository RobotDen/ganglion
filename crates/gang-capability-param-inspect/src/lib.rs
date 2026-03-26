//! Parameter server snapshot capability.
//!
//! Connects to a ROS 2 parameter server (via the ganglion:ros/interface broker),
//! snapshots all parameters, and optionally diffs against a reference snapshot.
//!
//! When compiled to a WASM component, this uses the WIT ros-interface import.
//! As a native library, the core logic is testable without ROS.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A snapshot of parameter values from one or more nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSnapshot {
    /// Timestamp when snapshot was taken (ISO 8601).
    pub timestamp: String,
    /// Parameters grouped by node name.
    pub nodes: BTreeMap<String, BTreeMap<String, ParamValue>>,
}

/// A parameter value with its type preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ParamValue {
    Bool(bool),
    Integer(i64),
    Double(f64),
    String(String),
    ByteArray(Vec<u8>),
    BoolArray(Vec<bool>),
    IntegerArray(Vec<i64>),
    DoubleArray(Vec<f64>),
    StringArray(Vec<String>),
}

/// A single difference between two parameter snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDiff {
    /// Node this parameter belongs to.
    pub node: String,
    /// Parameter name.
    pub param: String,
    /// What changed.
    pub change: DiffKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum DiffKind {
    /// Parameter exists in current but not reference.
    Added { value: ParamValue },
    /// Parameter exists in reference but not current.
    Removed { value: ParamValue },
    /// Parameter value changed.
    Changed {
        reference: ParamValue,
        current: ParamValue,
    },
}

/// Compute the diff between a reference snapshot and the current snapshot.
pub fn diff_snapshots(reference: &ParamSnapshot, current: &ParamSnapshot) -> Vec<ParamDiff> {
    let mut diffs = Vec::new();

    // Check all nodes/params in reference
    for (node, ref_params) in &reference.nodes {
        let cur_params = current.nodes.get(node);
        for (param, ref_val) in ref_params {
            match cur_params.and_then(|p| p.get(param)) {
                None => {
                    diffs.push(ParamDiff {
                        node: node.clone(),
                        param: param.clone(),
                        change: DiffKind::Removed {
                            value: ref_val.clone(),
                        },
                    });
                }
                Some(cur_val) if cur_val != ref_val => {
                    diffs.push(ParamDiff {
                        node: node.clone(),
                        param: param.clone(),
                        change: DiffKind::Changed {
                            reference: ref_val.clone(),
                            current: cur_val.clone(),
                        },
                    });
                }
                _ => {} // Same value
            }
        }
    }

    // Check for params in current that aren't in reference
    for (node, cur_params) in &current.nodes {
        let ref_params = reference.nodes.get(node);
        for (param, cur_val) in cur_params {
            let in_ref = ref_params.map(|p| p.contains_key(param)).unwrap_or(false);
            if !in_ref {
                diffs.push(ParamDiff {
                    node: node.clone(),
                    param: param.clone(),
                    change: DiffKind::Added {
                        value: cur_val.clone(),
                    },
                });
            }
        }
    }

    diffs
}

/// Format a snapshot as a human-readable table.
pub fn format_snapshot(snapshot: &ParamSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!("Parameter snapshot at {}\n", snapshot.timestamp));
    out.push_str(&"─".repeat(60));
    out.push('\n');

    for (node, params) in &snapshot.nodes {
        out.push_str(&format!("\n[{node}]\n"));
        for (param, value) in params {
            out.push_str(&format!("  {param} = {}\n", format_value(value)));
        }
    }
    out
}

/// Format a diff report.
pub fn format_diff(diffs: &[ParamDiff]) -> String {
    if diffs.is_empty() {
        return "No differences found.\n".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!("{} difference(s) found:\n", diffs.len()));
    out.push_str(&"─".repeat(60));
    out.push('\n');

    for diff in diffs {
        match &diff.change {
            DiffKind::Added { value } => {
                out.push_str(&format!(
                    "+ {}/{} = {}\n",
                    diff.node,
                    diff.param,
                    format_value(value)
                ));
            }
            DiffKind::Removed { value } => {
                out.push_str(&format!(
                    "- {}/{} = {}\n",
                    diff.node,
                    diff.param,
                    format_value(value)
                ));
            }
            DiffKind::Changed { reference, current } => {
                out.push_str(&format!(
                    "~ {}/{}: {} -> {}\n",
                    diff.node,
                    diff.param,
                    format_value(reference),
                    format_value(current)
                ));
            }
        }
    }
    out
}

fn format_value(v: &ParamValue) -> String {
    match v {
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Integer(i) => i.to_string(),
        ParamValue::Double(f) => format!("{f:.4}"),
        ParamValue::String(s) => format!("\"{s}\""),
        ParamValue::ByteArray(b) => format!("[{} bytes]", b.len()),
        ParamValue::BoolArray(a) => format!("{a:?}"),
        ParamValue::IntegerArray(a) => format!("{a:?}"),
        ParamValue::DoubleArray(a) => format!("{a:?}"),
        ParamValue::StringArray(a) => format!("{a:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot(timestamp: &str) -> ParamSnapshot {
        let mut nodes = BTreeMap::new();
        let mut params = BTreeMap::new();
        params.insert("max_speed".into(), ParamValue::Double(1.5));
        params.insert("use_sim".into(), ParamValue::Bool(false));
        params.insert("robot_name".into(), ParamValue::String("atlas".into()));
        nodes.insert("/robot_state".into(), params);

        let mut nav_params = BTreeMap::new();
        nav_params.insert("inflation_radius".into(), ParamValue::Double(0.55));
        nav_params.insert("costmap_width".into(), ParamValue::Integer(200));
        nodes.insert("/nav2".into(), nav_params);

        ParamSnapshot {
            timestamp: timestamp.into(),
            nodes,
        }
    }

    #[test]
    fn identical_snapshots_no_diff() {
        let snap = sample_snapshot("2026-04-23T12:00:00Z");
        let diffs = diff_snapshots(&snap, &snap);
        assert!(diffs.is_empty());
    }

    #[test]
    fn detect_changed_value() {
        let reference = sample_snapshot("2026-04-23T12:00:00Z");
        let mut current = sample_snapshot("2026-04-23T12:05:00Z");
        current
            .nodes
            .get_mut("/robot_state")
            .unwrap()
            .insert("max_speed".into(), ParamValue::Double(2.0));

        let diffs = diff_snapshots(&reference, &current);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].param, "max_speed");
        assert!(matches!(diffs[0].change, DiffKind::Changed { .. }));
    }

    #[test]
    fn detect_added_param() {
        let reference = sample_snapshot("2026-04-23T12:00:00Z");
        let mut current = sample_snapshot("2026-04-23T12:05:00Z");
        current
            .nodes
            .get_mut("/nav2")
            .unwrap()
            .insert("new_param".into(), ParamValue::Bool(true));

        let diffs = diff_snapshots(&reference, &current);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].param, "new_param");
        assert!(matches!(diffs[0].change, DiffKind::Added { .. }));
    }

    #[test]
    fn detect_removed_param() {
        let reference = sample_snapshot("2026-04-23T12:00:00Z");
        let mut current = sample_snapshot("2026-04-23T12:05:00Z");
        current
            .nodes
            .get_mut("/robot_state")
            .unwrap()
            .remove("use_sim");

        let diffs = diff_snapshots(&reference, &current);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].param, "use_sim");
        assert!(matches!(diffs[0].change, DiffKind::Removed { .. }));
    }

    #[test]
    fn detect_added_node() {
        let reference = sample_snapshot("2026-04-23T12:00:00Z");
        let mut current = sample_snapshot("2026-04-23T12:05:00Z");
        let mut new_params = BTreeMap::new();
        new_params.insert("enabled".into(), ParamValue::Bool(true));
        current.nodes.insert("/new_node".into(), new_params);

        let diffs = diff_snapshots(&reference, &current);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].node, "/new_node");
        assert!(matches!(diffs[0].change, DiffKind::Added { .. }));
    }

    #[test]
    fn format_snapshot_readable() {
        let snap = sample_snapshot("2026-04-23T12:00:00Z");
        let output = format_snapshot(&snap);
        assert!(output.contains("/robot_state"));
        assert!(output.contains("max_speed"));
        assert!(output.contains("1.5000"));
    }

    #[test]
    fn format_diff_readable() {
        let reference = sample_snapshot("2026-04-23T12:00:00Z");
        let mut current = sample_snapshot("2026-04-23T12:05:00Z");
        current
            .nodes
            .get_mut("/robot_state")
            .unwrap()
            .insert("max_speed".into(), ParamValue::Double(2.0));

        let diffs = diff_snapshots(&reference, &current);
        let output = format_diff(&diffs);
        assert!(output.contains("1 difference"));
        assert!(output.contains("max_speed"));
    }

    #[test]
    fn snapshot_json_roundtrip() {
        let snap = sample_snapshot("2026-04-23T12:00:00Z");
        let json = serde_json::to_string(&snap).unwrap();
        let loaded: ParamSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.nodes.len(), snap.nodes.len());
        assert_eq!(
            loaded.nodes["/robot_state"]["max_speed"],
            ParamValue::Double(1.5)
        );
    }
}
