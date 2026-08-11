//! WASM component entry point (wasm32 builds only).
//!
//! Records a time-bounded rosbag2 slice from inside the sandbox: builds the
//! `ros2 bag record` argv with the crate's canonical command builder, runs it
//! through the allowlisted `process-spawn` broker (wall-clock bounded), then
//! inventories the produced bag files through `fs-bounded` and, with
//! `--publish`, stores each file as a content-addressed artifact. Per-topic
//! message counts require bag introspection the sandbox intentionally cannot
//! do, so the report carries a single aggregate entry for the bag files.
//! Args: `[--start T] [--end T] [--format sqlite3|mcap] [--max-size MB]
//! [--topics a,b] [--duration-secs N] [--json] [--publish]`.

wit_bindgen::generate!({
    world: "ganglion-capability",
    path: "wit",
});

use ganglion::capability::{artifacts_publish, fs_bounded, process_spawn};

use crate::{TopicMetadata, build_record_command, build_result, format_report, parse_args};

/// Where the slice is recorded on the robot (must be within the FsBroker
/// jail and writable; the standard agent data dir qualifies).
const OUTPUT_DIR: &str = "/tmp/gang-rosbag-slice";
/// Default wall-clock bound on the recording subprocess.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

struct Component;

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        let json = args.iter().any(|a| a == "--json");
        let publish = args.iter().any(|a| a == "--publish");
        let mut timeout = DEFAULT_TIMEOUT_SECS;
        let mut filtered = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--json" | "--publish" => i += 1,
                "--duration-secs" if i + 1 < args.len() => {
                    timeout = args[i + 1].parse().unwrap_or(DEFAULT_TIMEOUT_SECS);
                    i += 2;
                }
                other => {
                    filtered.push(other.to_string());
                    i += 1;
                }
            }
        }

        let config = parse_args(&filtered)?;
        let argv = build_record_command(&config, OUTPUT_DIR);

        let result = process_spawn::spawn("ros2", &argv, timeout)?;
        // `ros2 bag record` is duration-bounded by the spawn timeout; a
        // timeout kill after capturing is the expected happy path, so only a
        // non-zero exit WITH empty output dir is treated as failure below.
        let stderr_tail = String::from_utf8_lossy(&result.stderr)
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .join(" | ");

        // Inventory what was produced.
        let files = fs_bounded::list_dir(OUTPUT_DIR).unwrap_or_default();
        let mut total_bytes: u64 = 0;
        let mut bag_files = 0u64;
        let mut first_cid: Option<String> = None;
        for name in &files {
            let path = format!("{OUTPUT_DIR}/{name}");
            if let Ok(stat) = fs_bounded::stat_file(&path) {
                if stat.is_file {
                    total_bytes += stat.size;
                    bag_files += 1;
                    if publish {
                        let data = fs_bounded::read_file(&path)?;
                        let cid = artifacts_publish::publish(
                            &data,
                            Some(name.as_str()),
                            Some("application/octet-stream"),
                        )?;
                        first_cid.get_or_insert(cid);
                    }
                }
            }
        }

        if bag_files == 0 {
            return Err(format!(
                "recording produced no bag files (ros2 exit {}; stderr: {})",
                result.exit_code,
                if stderr_tail.is_empty() {
                    "-"
                } else {
                    &stderr_tail
                }
            ));
        }

        let topics = vec![TopicMetadata {
            name: format!("(bag: {bag_files} file(s))"),
            message_type: "-".to_string(),
            message_count: 0,
            size_bytes: total_bytes,
        }];
        let mut slice = build_result(&config, topics, "n/a", "n/a", timeout as f64);
        slice.cid = first_cid;

        if json {
            serde_json::to_vec(&slice).map_err(|e| e.to_string())
        } else {
            Ok(format_report(&slice).into_bytes())
        }
    }
}

export!(Component);
