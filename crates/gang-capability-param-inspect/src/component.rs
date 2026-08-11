//! WASM component entry point (wasm32 builds only).
//!
//! Snapshots ROS 2 parameters through the `ros-interface` import. Each arg
//! names a parameter as `<node>:<param>` (a bare name is grouped under
//! `(unqualified)`); the raw value bytes the broker returns are recorded as
//! string parameter values in the snapshot. Diffing against a reference
//! snapshot remains a host-side workflow (feed two `--json` snapshots to
//! `diff_snapshots`); the component's job is producing the current snapshot
//! from inside the sandbox. Args: `<node:param> [...] [--json]`.

use std::collections::BTreeMap;

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::ros_interface;

use crate::{ParamSnapshot, ParamValue, format_snapshot};

struct Component;

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let json = args.iter().any(|a| a == "--json");
        let names: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
        if names.is_empty() {
            return Err(
                "usage: <node:param> [node:param ...] [--json] — no parameters requested"
                    .to_string(),
            );
        }

        let mut nodes: BTreeMap<String, BTreeMap<String, ParamValue>> = BTreeMap::new();
        for name in names {
            let value_bytes = ros_interface::param_get(name)?;
            let value = String::from_utf8_lossy(&value_bytes).trim().to_string();
            let (node, param) = match name.split_once(':') {
                Some((n, p)) => (n.to_string(), p.to_string()),
                None => ("(unqualified)".to_string(), name.to_string()),
            };
            nodes
                .entry(node)
                .or_default()
                .insert(param, ParamValue::String(value));
        }

        let snapshot = ParamSnapshot {
            timestamp: "n/a".to_string(),
            nodes,
        };

        if json {
            serde_json::to_vec(&snapshot).map_err(|e| e.to_string())
        } else {
            Ok(format_snapshot(&snapshot).into_bytes())
        }
    }
}

export!(Component);
