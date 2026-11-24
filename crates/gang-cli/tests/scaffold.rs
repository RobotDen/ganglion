//! The Rust capability scaffold and `gang sign --capabilities none`.
//!
//! Building and running a scaffolded component needs a WASM toolchain and a
//! relay, so that is `scripts/check-scaffold.sh` (a CI job). These tests pin
//! what the scaffold writes and the signing it relies on.

use gang_core::identity::Keypair;
use gang_core::manifest::SignedManifest;
use std::path::Path;
use std::process::Command;

fn gang(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gang"))
        .args(args)
        .env("GANG_HOME", home)
        .env("GANG_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawning `gang`")
}

#[test]
fn sign_capabilities_none_declares_no_group() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("identity.key");
    Keypair::load_or_generate(&key).unwrap();
    let wasm = dir.path().join("tool.wasm");
    std::fs::write(&wasm, b"fixture").unwrap();

    let out = gang(
        dir.path(),
        &[
            "sign",
            wasm.to_str().unwrap(),
            "--key",
            key.to_str().unwrap(),
            "--capabilities",
            "none",
        ],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "gang sign failed: {stderr}");
    assert!(
        !stderr.contains("Falling back to a permissive default"),
        "`none` is an explicit declaration, not a fallback: {stderr}"
    );

    let cbor = std::fs::read(dir.path().join("tool.manifest.cbor")).unwrap();
    let manifest = SignedManifest::from_cbor(&cbor)
        .unwrap()
        .verify_and_decode()
        .unwrap();
    assert!(manifest.declared_capabilities.is_empty());
}

#[test]
fn rust_scaffold_exports_the_capability_world() {
    let dir = tempfile::tempdir().unwrap();
    let out = gang(
        dir.path(),
        &[
            "capability",
            "scaffold",
            "my-tool",
            "--output-dir",
            dir.path().to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let project = dir.path().join("my-tool");
    let read = |p: &str| std::fs::read_to_string(project.join(p)).unwrap();

    let component = read("src/component.rs");
    assert!(component.contains("world: \"ganglion-capability\""));
    assert!(component.contains("export!(Component)"));
    assert!(read("src/lib.rs").contains("mod component;"));
    assert!(read("wit/ganglion.wit").contains("world ganglion-capability"));

    // The toolchain the runtime can run (WASI 0.2.6 host): see
    // SCAFFOLD_RUST_TOOLCHAIN.
    assert!(read("rust-toolchain.toml").contains("channel = \"1.88\""));

    let cargo: toml::Table = read("Cargo.toml").parse().unwrap();
    assert!(cargo.contains_key("workspace"), "standalone project");
    let workspace: toml::Table =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
    assert_eq!(
        cargo["target"]["cfg(target_arch = \"wasm32\")"]["dependencies"]["wit-bindgen"],
        workspace["workspace"]["dependencies"]["wit-bindgen"],
        "the scaffold uses the workspace's wit-bindgen"
    );

    let makefile = read("Makefile");
    assert!(
        makefile.contains("cp target/wasm32-wasip2/release/my_tool.wasm my-tool.component.wasm")
    );
    assert!(makefile.contains("--capabilities none"));
}
