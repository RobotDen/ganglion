//! WASM component entry point (wasm32 builds only).
//!
//! Streams log lines from a host log source via `logs-stream`, runs them
//! through the crate's canonical normalizer (journald / ROS 2 / syslog
//! auto-detection), and returns the normalization report. Args:
//! `[source] [--pattern <p>] [--json]` — with no source, the first source the
//! host advertises is used.

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::logs_stream;

use crate::{format_report, normalize_batch};

struct Component;

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let mut source: Option<String> = None;
        let mut pattern = String::new();
        let mut json = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--pattern" | "-p" if i + 1 < args.len() => {
                    pattern = args[i + 1].clone();
                    i += 2;
                }
                "--json" => {
                    json = true;
                    i += 1;
                }
                s if !s.starts_with('-') => {
                    source = Some(s.to_string());
                    i += 1;
                }
                _ => i += 1,
            }
        }

        let source = match source {
            Some(s) => s,
            None => {
                let sources = logs_stream::list_sources()?;
                sources
                    .first()
                    .map(|s| s.name.clone())
                    .ok_or_else(|| "no log sources available on this robot".to_string())?
            }
        };

        let lines = logs_stream::stream_logs(&source, &pattern)?;
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let report = normalize_batch(&refs);

        if json {
            serde_json::to_vec(&report).map_err(|e| e.to_string())
        } else {
            Ok(format_report(&report).into_bytes())
        }
    }
}

export!(Component);
