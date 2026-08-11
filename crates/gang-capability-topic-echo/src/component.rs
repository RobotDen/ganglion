//! WASM component entry point (wasm32 builds only).
//!
//! Captures ROS 2 topic data through the `ros-interface` import and runs it
//! through the crate's canonical decimation/report pipeline. The broker's
//! `topic-subscribe` currently returns one serialized snapshot per call, so
//! each requested topic contributes a single raw message per invocation —
//! decimation and per-topic caps still apply and the report shape is
//! identical to a multi-message capture. With `--publish` the JSON report is
//! stored as a content-addressed artifact and the CID is appended. Args:
//! `topic [topic ...] [--decimation N] [--max N] [--json] [--publish]`.

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::{artifacts_publish, ros_interface};

use crate::{build_report, decimate, format_report, parse_args};

struct Component;

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let json = args.iter().any(|a| a == "--json");
        let publish = args.iter().any(|a| a == "--publish");
        let filtered: Vec<String> = args
            .iter()
            .filter(|a| *a != "--json" && *a != "--publish")
            .cloned()
            .collect();
        let config = parse_args(&filtered);

        if config.topics.is_empty() {
            // Help the operator: list what's actually available.
            let listing = ros_interface::list_ros().unwrap_or_default();
            let topics: Vec<String> = listing
                .into_iter()
                .filter(|e| e.entry_type == "topic")
                .map(|e| e.name)
                .collect();
            return Err(format!(
                "no topics requested. usage: topic [...] [--decimation N] [--max N]. \
                 available topics: {}",
                if topics.is_empty() {
                    "(none visible)".to_string()
                } else {
                    topics.join(", ")
                }
            ));
        }

        let mut results = Vec::new();
        for topic in &config.topics {
            let raw = ros_interface::topic_subscribe(topic)?;
            results.push(decimate(
                topic,
                &[raw],
                config.decimation,
                config.max_messages,
                "n/a",
            ));
        }

        let report = build_report(&config, results);

        let mut out = if json {
            serde_json::to_vec(&report).map_err(|e| e.to_string())?
        } else {
            format_report(&report).into_bytes()
        };

        if publish {
            let payload = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
            let cid = artifacts_publish::publish(
                &payload,
                Some("topic-echo-report.json"),
                Some("application/json"),
            )?;
            out.extend_from_slice(format!("\nartifact: {cid}\n").as_bytes());
        }

        Ok(out)
    }
}

export!(Component);
