//! Test diagnostics WASM component for Ganglion e2e testing.
//!
//! This component imports the `diagnostics-collect` interface and exports
//! the `run` function. It collects system info and returns a JSON report.

mod bindings;

use bindings::ganglion::capability::diagnostics_collect;
use bindings::Guest;

struct TestDiagnostics;

impl Guest for TestDiagnostics {
    fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
        // Collect system info from the host
        let sys_info = diagnostics_collect::system_info()
            .map_err(|e| format!("system_info failed: {e}"))?;

        // Build a simple JSON report
        let mut report = serde_json::Map::new();
        report.insert(
            "component".into(),
            serde_json::Value::String("test-diagnostics".into()),
        );
        report.insert(
            "version".into(),
            serde_json::Value::String("0.1.0".into()),
        );

        // Convert diagnostic entries to JSON
        let mut sys = serde_json::Map::new();
        for entry in &sys_info {
            sys.insert(
                entry.key.clone(),
                serde_json::Value::String(entry.value.clone()),
            );
        }
        report.insert("system_info".into(), serde_json::Value::Object(sys));

        if !args.is_empty() {
            report.insert(
                "args".into(),
                serde_json::Value::Array(
                    args.into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
        }

        let json = serde_json::to_vec_pretty(&serde_json::Value::Object(report))
            .map_err(|e| format!("JSON serialization failed: {e}"))?;

        Ok(json)
    }
}

bindings::export!(TestDiagnostics with_types_in bindings);
