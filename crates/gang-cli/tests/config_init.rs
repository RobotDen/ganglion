//! Integration tests for `gang config init --force` and `gang init --force`.
//!
//! Both reset the operator config to its defaults. They must not erase the
//! telemetry opt-out written by `gang telemetry off` (doing so silently turns
//! telemetry back on), nor other settings the operator config doesn't own.

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

fn telemetry_enabled(cfg: &toml::Table) -> Option<bool> {
    cfg.get("telemetry")
        .and_then(|t| t.get("enabled"))
        .and_then(|v| v.as_bool())
}

/// Opt out, add a key the operator config doesn't own, and change one it
/// does; then run `force_args` and check what survived.
fn assert_force_keeps_opt_out(force_args: &[&str]) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let (out, ok) = gang(home, &["telemetry", "off"]);
    assert!(ok, "`gang telemetry off` should succeed:\n{out}");
    let (out, ok) = gang(home, &["config", "set", "host_key_policy", "tofu"]);
    assert!(ok, "`gang config set` should succeed:\n{out}");
    let mut cfg = read_config(home);
    cfg.insert("user_note".into(), "keep me".into());
    std::fs::write(
        home.join("config.toml"),
        toml::to_string_pretty(&cfg).unwrap(),
    )
    .unwrap();

    let (out, ok) = gang(home, force_args);
    assert!(ok, "`gang {}` should succeed:\n{out}", force_args.join(" "));

    let cfg = read_config(home);
    assert_eq!(
        telemetry_enabled(&cfg),
        Some(false),
        "the telemetry opt-out must survive `gang {}`:\n{cfg:#?}",
        force_args.join(" ")
    );
    assert_eq!(
        cfg.get("user_note").and_then(|v| v.as_str()),
        Some("keep me"),
        "keys the operator config doesn't own must survive:\n{cfg:#?}"
    );
    assert_eq!(
        cfg.get("host_key_policy").and_then(|v| v.as_str()),
        Some("strict"),
        "--force resets the operator settings to their defaults:\n{cfg:#?}"
    );
}

#[test]
fn config_init_force_keeps_telemetry_opt_out() {
    assert_force_keeps_opt_out(&["config", "init", "--force"]);
}

#[test]
fn init_force_keeps_telemetry_opt_out() {
    assert_force_keeps_opt_out(&["init", "--force", "--yes"]);
}

/// The malformed-config warnings tell users to run `gang config init
/// --force`. The old file can't be kept, so neither can its telemetry
/// setting: the new file turns telemetry off rather than silently on.
#[test]
fn config_init_force_on_malformed_file_turns_telemetry_off() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    std::fs::write(
        home.join("config.toml"),
        "[telemetry]\nenabled = false\nthis is not toml\n",
    )
    .unwrap();

    let (out, ok) = gang(home, &["config", "init", "--force"]);
    assert!(ok, "`gang config init --force` should succeed:\n{out}");
    assert!(
        out.contains("telemetry is now off"),
        "the user must be told telemetry was turned off:\n{out}"
    );

    let cfg = read_config(home);
    assert_eq!(telemetry_enabled(&cfg), Some(false), "{cfg:#?}");
    assert_eq!(
        cfg.get("host_key_policy").and_then(|v| v.as_str()),
        Some("strict"),
        "{cfg:#?}"
    );
}

/// Without an existing file, `config init` writes plain defaults and leaves
/// telemetry to its normal (documented) default.
#[test]
fn config_init_without_a_file_writes_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let (out, ok) = gang(home, &["config", "init"]);
    assert!(ok, "`gang config init` should succeed:\n{out}");

    let cfg = read_config(home);
    assert_eq!(telemetry_enabled(&cfg), None, "{cfg:#?}");
    assert_eq!(
        cfg.get("host_key_policy").and_then(|v| v.as_str()),
        Some("strict"),
        "{cfg:#?}"
    );
}
