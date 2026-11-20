//! Integration tests for `gang config set`.
//!
//! `config set` must edit config.toml in place: changing one key may not drop
//! anything else in the file, in particular `[telemetry] enabled = false`
//! written by `gang telemetry off` (dropping it silently re-enables
//! telemetry). A malformed file must be left untouched, with an error.

use std::path::Path;
use std::process::Command;

/// Run `gang <args>` against a scratch GANG_HOME; return (stdout+stderr, success).
fn gang(home: &Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_gang"))
        .args(args)
        .env("GANG_HOME", home)
        // Never let a test run reach the telemetry endpoint.
        .env("GANG_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawning `gang`");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (text, out.status.success())
}

fn read_config(home: &Path) -> toml::Table {
    std::fs::read_to_string(home.join("config.toml"))
        .expect("config.toml exists")
        .parse()
        .expect("config.toml is valid TOML")
}

#[test]
fn config_set_keeps_telemetry_opt_out() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let (out, ok) = gang(home, &["telemetry", "off"]);
    assert!(ok, "`gang telemetry off` should succeed:\n{out}");

    let (out, ok) = gang(
        home,
        &["config", "set", "default_relay", "/ip4/192.0.2.1/tcp/4001"],
    );
    assert!(ok, "`gang config set` should succeed:\n{out}");

    let cfg = read_config(home);
    assert_eq!(
        cfg.get("telemetry")
            .and_then(|t| t.get("enabled"))
            .and_then(|v| v.as_bool()),
        Some(false),
        "the telemetry opt-out must survive `config set`:\n{cfg:#?}"
    );
    assert_eq!(
        cfg.get("default_relay").and_then(|v| v.as_str()),
        Some("/ip4/192.0.2.1/tcp/4001")
    );
}

#[test]
fn config_set_refuses_to_overwrite_malformed_file() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let path = home.join("config.toml");
    let original = "[telemetry]\nenabled = false\nthis is = = not toml\n";
    std::fs::write(&path, original).unwrap();

    let (out, ok) = gang(home, &["config", "set", "host_key_policy", "tofu"]);
    assert!(!ok, "`config set` must fail on a malformed file:\n{out}");
    assert!(
        out.contains("config.toml"),
        "the error should name the file:\n{out}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        original,
        "a malformed config.toml must be left byte-for-byte untouched"
    );
}
