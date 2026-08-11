//! End-to-end validation of REAL componentized capability crates (#28).
//!
//! These tests load an actual `cargo build --target wasm32-wasip2` component
//! (not a WAT fixture) and run it through the full runtime — linker, WASI
//! deny-set, capability imports, fuel/deadline limits — against mock brokers.
//!
//! They are gated on the `GANG_COMPONENT_DIR` env var (a directory containing
//! `gang_capability_*.wasm`) because the wasm artifacts are build outputs, not
//! checked-in fixtures: without the var the tests pass as skipped no-ops, so
//! CI and the normal gate are unaffected. Run explicitly with:
//!
//! ```sh
//! cargo build -p gang-capability-diagnostics --release --target wasm32-wasip2
//! GANG_COMPONENT_DIR=target/wasm32-wasip2/release cargo test -p gang-wasm-host --test real_component
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use gang_core::broker::{CapabilityBroker, CapabilityRequest, CapabilityResponse};
use gang_core::capability::CapabilityGroup;
use gang_core::error::BrokerError;
use gang_core::manifest::ResourceLimits;
use gang_wasm_host::GanglionEngine;
use gang_wasm_host::runtime::ComponentRuntime;

/// Mock diagnostics broker returning a realistic SystemInfo JSON object —
/// the same shape the real broker serializes, which the host flattens into
/// `diagnostic-entry` key/value pairs.
struct MockDiagnostics;

#[async_trait::async_trait]
impl CapabilityBroker for MockDiagnostics {
    async fn handle_request(
        &self,
        _req: CapabilityRequest,
    ) -> Result<CapabilityResponse, BrokerError> {
        let data = serde_json::json!({
            "hostname": "test-robot-7",
            "os": "Ubuntu",
            "os_version": "24.04",
            "arch": "aarch64",
            "uptime_secs": 86400,
            "cpu_count": 8,
            "memory_total_bytes": 8589934592u64,
            "memory_available_bytes": 4294967296u64,
            "disk_total_bytes": 107374182400u64,
            "disk_available_bytes": 53687091200u64,
            "ganglion_version": "2.2.0",
        })
        .to_string()
        .into_bytes();
        let bytes_out = data.len() as u64;
        Ok(CapabilityResponse {
            success: true,
            data,
            error: None,
            bytes_in: 0,
            bytes_out,
        })
    }

    fn capability_group(&self) -> &str {
        "ganglion:diagnostics/collect"
    }
}

fn component_bytes(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var("GANG_COMPONENT_DIR").ok()?;
    std::fs::read(format!("{dir}/{name}")).ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn real_diagnostics_component_runs_end_to_end() {
    let Some(bytes) = component_bytes("gang_capability_diagnostics.wasm") else {
        eprintln!("skipped: set GANG_COMPONENT_DIR to a dir with built components");
        return;
    };

    let engine = GanglionEngine::new().unwrap();
    let mut brokers: HashMap<String, Arc<dyn CapabilityBroker>> = HashMap::new();
    brokers.insert(
        "ganglion:diagnostics/collect".into(),
        Arc::new(MockDiagnostics),
    );
    let runtime = ComponentRuntime::new(engine, brokers).unwrap();

    // Text mode: the formatted report must reflect the mock's data.
    let result = runtime
        .invoke(
            &bytes,
            "real-diagnostics",
            vec![CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            }],
            &ResourceLimits::default(),
            vec![],
        )
        .await
        .expect("real diagnostics component must instantiate and run");
    let text = String::from_utf8_lossy(&result.data);
    assert!(text.contains("test-robot-7"), "hostname missing: {text}");
    assert!(text.contains("Ubuntu 24.04"), "os missing: {text}");
    assert!(text.contains("8192 MB"), "memory MB missing: {text}");
    assert!(text.contains("50.0% used"), "disk pct missing: {text}");

    // JSON mode round-trips through serde.
    let result = runtime
        .invoke(
            &bytes,
            "real-diagnostics-json",
            vec![CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            }],
            &ResourceLimits::default(),
            vec!["--json".into()],
        )
        .await
        .expect("json mode must run");
    let v: serde_json::Value = serde_json::from_slice(&result.data).expect("valid JSON");
    assert_eq!(v["hostname"], "test-robot-7");
    assert_eq!(v["memory_total_mb"], 8192);
}

#[tokio::test(flavor = "multi_thread")]
async fn real_component_undeclared_capability_is_denied() {
    let Some(bytes) = component_bytes("gang_capability_diagnostics.wasm") else {
        eprintln!("skipped: set GANG_COMPONENT_DIR to a dir with built components");
        return;
    };

    let engine = GanglionEngine::new().unwrap();
    let mut brokers: HashMap<String, Arc<dyn CapabilityBroker>> = HashMap::new();
    brokers.insert(
        "ganglion:diagnostics/collect".into(),
        Arc::new(MockDiagnostics),
    );
    let runtime = ComponentRuntime::new(engine, brokers).unwrap();

    // No declared capabilities: the component instantiates (all imports are
    // linked) but its system-info call must be DENIED at call time, which the
    // guest surfaces as a run error.
    let result = runtime
        .invoke(
            &bytes,
            "real-diagnostics-denied",
            vec![],
            &ResourceLimits::default(),
            vec![],
        )
        .await;
    match result {
        Ok(r) => panic!(
            "undeclared capability should not succeed: {}",
            String::from_utf8_lossy(&r.data)
        ),
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("not declared") || msg.contains("denied") || msg.contains("declare"),
                "expected a denial error, got: {msg}"
            );
        }
    }
}
