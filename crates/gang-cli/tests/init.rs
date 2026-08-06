//! Integration test for `gang init` — guided first-run setup.
//!
//! `gang init` is a fast, non-blocking command (unlike `gang up`), so this
//! test shells out to the built binary and drives it against a scratch data
//! directory via the global `--data-dir` flag (no docker, no network beyond the
//! local archetype probes). It proves the three first-run guarantees:
//!   1. one non-interactive run produces an identity, a default-deny policy, and
//!      an operator config;
//!   2. a second run without `--force` is non-destructive (keys/policy/config
//!      are kept, not clobbered); and
//!   3. `--json` emits a single valid JSON object describing the setup.

use std::path::Path;
use std::process::Command;

/// Run `gang --data-dir <dir> init <extra-args>` non-interactively and return
/// (stdout, success). Stdin is inherited from the test harness (not a TTY), so
/// even without `--yes` this would run non-interactively; we pass `--yes` to be
/// explicit and deterministic.
fn run_init(dir: &Path, extra: &[&str]) -> (String, bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gang"));
    cmd.arg("--data-dir").arg(dir).arg("init");
    cmd.args(extra);
    // Keep the archetype probes from touching the real user's ~/.gang.
    cmd.env("GANG_HOME", dir);
    let out = cmd.output().expect("spawning `gang init`");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (stdout, out.status.success())
}

#[test]
fn init_creates_identity_policy_and_config() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let (stdout, ok) = run_init(dir, &["--yes"]);
    assert!(ok, "first `gang init --yes` should succeed:\n{stdout}");

    let key = dir.join("identity.key");
    let policy = dir.join("policy.toml");
    let config = dir.join("config.toml");
    assert!(key.exists(), "identity key should be created");
    assert!(policy.exists(), "policy.toml should be created");
    assert!(config.exists(), "config.toml should be created");

    // The policy must be genuinely default-deny: it parses, authorizes exactly
    // one deploying peer, and permits ZERO capability groups (every example
    // capability rule ships commented out).
    let policy_text = std::fs::read_to_string(&policy).unwrap();
    let parsed = gang_core::policy::Policy::from_toml(&policy_text)
        .expect("generated policy.toml must be valid");
    assert!(
        parsed.capability_rules.is_empty(),
        "default-deny policy must permit no capability groups by default"
    );
    assert_eq!(
        parsed.peer_rules.len(),
        1,
        "policy should authorize exactly the operator to deploy"
    );
    assert!(parsed.peer_rules[0].can_deploy);
    assert!(
        policy_text.contains("DEFAULT DENY"),
        "policy should be clearly labeled default-deny"
    );
    assert!(
        policy_text.contains("# group = \"ganglion:diagnostics/collect\""),
        "policy should carry commented example rules to uncomment"
    );

    // The config must carry the sane default host-key policy.
    let config_text = std::fs::read_to_string(&config).unwrap();
    assert!(
        config_text.contains("host_key_policy = \"strict\""),
        "config should default host_key_policy to strict"
    );

    // The identity peer id is the one authorized in the policy.
    let kp = gang_core::identity::Keypair::load(&key).unwrap();
    assert_eq!(parsed.peer_rules[0].peer_id, kp.peer_id().to_string());
}

#[test]
fn second_run_without_force_is_non_destructive() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let (_out1, ok1) = run_init(dir, &["--yes"]);
    assert!(ok1);
    let key_before = std::fs::read(dir.join("identity.key")).unwrap();
    let policy_before = std::fs::read(dir.join("policy.toml")).unwrap();
    let config_before = std::fs::read(dir.join("config.toml")).unwrap();

    let (out2, ok2) = run_init(dir, &["--yes"]);
    assert!(ok2, "second `gang init` should succeed and be idempotent");
    assert!(
        out2.contains("Already present") && out2.contains("kept"),
        "second run must report the existing identity/policy/config it kept:\n{out2}"
    );

    // Nothing was clobbered.
    assert_eq!(
        key_before,
        std::fs::read(dir.join("identity.key")).unwrap(),
        "identity key must be preserved across re-runs without --force"
    );
    assert_eq!(
        policy_before,
        std::fs::read(dir.join("policy.toml")).unwrap(),
        "policy must be preserved without --force"
    );
    assert_eq!(
        config_before,
        std::fs::read(dir.join("config.toml")).unwrap(),
        "config must be preserved without --force"
    );
}

#[test]
fn json_output_is_valid_and_reports_setup() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let (stdout, ok) = run_init(dir, &["--json"]);
    assert!(ok, "`gang init --json` should succeed:\n{stdout}");

    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("`gang init --json` must emit valid JSON");
    assert_eq!(value["status"], "configured");
    assert!(value["archetype"]["name"].is_string());
    assert!(value["identity"]["id"].is_string());
    assert_eq!(value["identity"]["created"], true);
    assert!(value["policy_path"].is_string());
    assert!(value["config_path"].is_string());
    assert!(
        value["next_commands"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "JSON should list the next commands to run"
    );
    // The first next step is always the loopback fleet.
    assert_eq!(value["next_commands"][0], "gang up");
}
