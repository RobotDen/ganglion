//! `gang sign` resource-limit flags (#48): --cpu-fuel, --wall-clock-secs,
//! and --max-memory-bytes must land verbatim in the signed manifest, and
//! omitting them must leave the limits at zero (= host defaults, per #49).
//!
//! Drives the real binary via CARGO_BIN_EXE_gang so the flag parsing,
//! plumbing, and manifest serialization are all covered end-to-end.

use gang_core::identity::Keypair;
use gang_core::manifest::SignedManifest;
use std::process::Command;

fn run_sign(dir: &std::path::Path, extra: &[&str]) -> gang_core::manifest::ComponentManifest {
    let key_path = dir.join("identity.key");
    Keypair::load_or_generate(&key_path).expect("test key generates");
    let wasm_path = dir.join("tool.wasm");
    std::fs::write(&wasm_path, b"not really wasm - broker path fixture").unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gang"));
    // Keep the test hermetic: no telemetry accumulation or notice noise.
    cmd.env("GANG_TELEMETRY", "off");
    cmd.arg("sign")
        .arg(&wasm_path)
        .args(["--key", key_path.to_str().unwrap()])
        .args(["--capabilities", "diagnostics"])
        .args(extra);
    let output = cmd.output().expect("gang sign runs");
    assert!(
        output.status.success(),
        "gang sign failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cbor = std::fs::read(dir.join("tool.manifest.cbor")).expect("manifest written");
    SignedManifest::from_cbor(&cbor)
        .expect("manifest parses")
        .verify_and_decode()
        .expect("signature verifies")
}

#[test]
fn sign_writes_resource_limit_flags_into_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = run_sign(
        dir.path(),
        &[
            "--cpu-fuel",
            "5000000000",
            "--wall-clock-secs",
            "600",
            "--max-memory-bytes",
            "536870912",
        ],
    );
    assert_eq!(manifest.limits.cpu_fuel, 5_000_000_000);
    assert_eq!(manifest.limits.wall_clock_secs, 600);
    assert_eq!(manifest.limits.max_memory_bytes, 536_870_912);
}

#[test]
fn sign_defaults_limits_to_zero_meaning_host_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = run_sign(dir.path(), &[]);
    // Zeros mean "host default, clamped to the hard cap" (#49) — the
    // runtime's effective_* helpers apply them; the manifest stays honest.
    assert_eq!(manifest.limits.cpu_fuel, 0);
    assert_eq!(manifest.limits.wall_clock_secs, 0);
    assert_eq!(manifest.limits.max_memory_bytes, 0);
}
