//! Anonymous, opt-out, operator-side-only telemetry (ADR-026).
//!
//! Everything here is deliberately boring and inspectable:
//!
//! - **Never on robots, never in customer networks.** Only the operator
//!   commands in [`ALLOWED_COMMANDS`] can touch this module. `agent`, `join`,
//!   and `relay` (the processes that run on robots and infrastructure) are
//!   excluded, as are the field-triage commands (`doctor`, `diagnose`,
//!   `test-archetype`) that are frequently run *inside* customer networks.
//!   If you operate Ganglion in production, disable telemetry outright —
//!   see `TELEMETRY.md` at the repository root.
//! - **One request per day, maximum.** Commands increment a local counter;
//!   the first allowlisted command of a UTC day flushes the aggregate in a
//!   single request that doubles as an update check. There is no event
//!   stream and no per-command ping.
//! - **The payload is exhaustively listed** in the `Payload` struct and printable
//!   before it is ever sent: `gang telemetry show`.
//! - **Silent failure, never retry, never block.** A blocked or missing
//!   endpoint changes nothing about any command's output or exit code.
//! - **Anonymous.** The id is a random UUID, never derived from the machine
//!   or the gang identity, resettable with `gang telemetry reset`.
//!
//! Opt-out layers (any one suffices): `DO_NOT_TRACK`, `GANG_TELEMETRY=off`,
//! `CI`, `[telemetry] enabled = false` in config.toml, `gang telemetry off`,
//! or building without the `telemetry` cargo feature (no code at all).

use crate::Commands;

/// Operator commands that may record telemetry. Everything absent — most
/// notably `agent`, `join`, `relay`, `doctor`, `diagnose`, `test-archetype`,
/// `demo`, `up`, `mcp` — never touches this module (ADR-026 layer 1).
pub const ALLOWED_COMMANDS: &[&str] = &[
    "init",
    "status",
    "deploy",
    "run",
    "caps",
    "peer",
    "policy",
    "registry",
    "sign",
    "view",
    "tui",
    "logs",
    "pair",
    "profiles",
    "alert",
    "config",
    "capability",
    "telemetry",
];

/// Map a parsed command to its telemetry category — the top-level subcommand
/// name only, never arguments. Returns a name even for non-allowlisted
/// commands; [`record_command`] applies the allowlist.
pub fn command_category(command: &Commands) -> &'static str {
    match command {
        Commands::Init { .. } => "init",
        Commands::Pair { .. } => "pair",
        Commands::Join { .. } => "join",
        Commands::Identity { .. } => "identity",
        Commands::Sign { .. } => "sign",
        Commands::Agent { .. } => "agent",
        Commands::Deploy { .. } => "deploy",
        Commands::Run { .. } => "run",
        Commands::Caps { .. } => "caps",
        Commands::Logs { .. } => "logs",
        Commands::Demo => "demo",
        Commands::Up { .. } => "up",
        Commands::TestArchetype { .. } => "test-archetype",
        Commands::Diagnose { .. } => "diagnose",
        Commands::Doctor { .. } => "doctor",
        Commands::Profiles => "profiles",
        Commands::Alert { .. } => "alert",
        Commands::Mcp => "mcp",
        Commands::TransportStats { .. } => "transport-stats",
        Commands::Fetch { .. } => "fetch",
        Commands::Push { .. } => "push",
        Commands::Artifacts => "artifacts",
        Commands::Capability { .. } => "capability",
        Commands::New { .. } => "new",
        Commands::Registry { .. } => "registry",
        Commands::Peer { .. } => "peer",
        Commands::Policy { .. } => "policy",
        Commands::Config { .. } => "config",
        Commands::Status { .. } => "status",
        Commands::List => "list",
        Commands::Tui { .. } => "tui",
        Commands::Connect { .. } => "connect",
        Commands::View { .. } => "view",
        Commands::Completions { .. } => "completions",
        Commands::Relay { .. } => "relay",
        Commands::Telemetry { .. } => "telemetry",
    }
}

#[cfg(feature = "telemetry")]
pub use imp::{record_command, telemetry_cli};

#[cfg(not(feature = "telemetry"))]
/// Telemetry compiled out (`--no-default-features`): a no-op.
pub fn record_command(_category: &str, _ok: bool, _notify: bool) {}

#[cfg(not(feature = "telemetry"))]
/// `gang telemetry` when compiled out: report that, truthfully.
pub fn telemetry_cli(_action: &crate::TelemetryAction) -> anyhow::Result<()> {
    println!(
        "Telemetry is COMPILED OUT of this binary (built without the `telemetry` \
         cargo feature). Nothing is collected and nothing can be enabled."
    );
    Ok(())
}

#[cfg(feature = "telemetry")]
mod imp {
    use super::ALLOWED_COMMANDS;
    use crate::TelemetryAction;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// The single endpoint. Documented in TELEMETRY.md so a security team can
    /// block it host-wide with zero functional impact (blocked = silent no-op).
    const ENDPOINT: &str = "https://checkpoint.robotden.dev/v1/checkpoint";

    /// Whole-request deadline: one command per day pays at most this.
    const SEND_TIMEOUT: Duration = Duration::from_secs(2);

    /// The COMPLETE daily payload (ADR-026: this list is exhaustive; adding a
    /// field requires amending the ADR and TELEMETRY.md, and bumping `schema`).
    #[derive(Debug, Serialize, Deserialize)]
    pub struct Payload {
        /// Payload schema version.
        pub schema: u32,
        /// Random UUID — never machine- or identity-derived. `gang telemetry
        /// reset` regenerates it.
        pub id: String,
        /// CLI version.
        pub version: String,
        /// OS family (`std::env::consts::OS`).
        pub os: String,
        /// CPU architecture (`std::env::consts::ARCH`).
        pub arch: String,
        /// Distribution channel baked in at release build; "source" otherwise.
        pub dist: String,
        /// Per-command-category success/error counts since the last flush.
        pub counts: BTreeMap<String, CategoryCount>,
    }

    /// Success/error tally for one command category.
    #[derive(Debug, Default, Clone, Serialize, Deserialize)]
    pub struct CategoryCount {
        /// Invocations that exited successfully.
        pub ok: u64,
        /// Invocations that returned an error.
        pub err: u64,
    }

    /// Why telemetry is disabled, when it is — named so `gang telemetry
    /// status` can tell the user exactly which layer applied.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Disposition {
        Enabled,
        DisabledBy(&'static str),
    }

    /// Evaluate every opt-out layer, in documented order.
    pub fn disposition() -> Disposition {
        if std::env::var_os("DO_NOT_TRACK").is_some_and(|v| !v.is_empty()) {
            return Disposition::DisabledBy("DO_NOT_TRACK environment variable");
        }
        if std::env::var("GANG_TELEMETRY").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
            return Disposition::DisabledBy("GANG_TELEMETRY=off environment variable");
        }
        if std::env::var_os("CI").is_some_and(|v| !v.is_empty()) {
            return Disposition::DisabledBy("CI environment variable");
        }
        if config_disabled() {
            return Disposition::DisabledBy("[telemetry] enabled = false in config.toml");
        }
        Disposition::Enabled
    }

    fn config_disabled() -> bool {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            return false;
        };
        value
            .get("telemetry")
            .and_then(|t| t.get("enabled"))
            .and_then(|e| e.as_bool())
            == Some(false)
    }

    fn state_dir() -> PathBuf {
        gang_core::identity::default_config_dir().join("telemetry")
    }

    /// Record one command outcome, and — on the first allowlisted command of
    /// a UTC day, after the disclosure notice has been shown — flush the
    /// aggregate as the daily checkpoint. Failures everywhere are silent by
    /// design: telemetry must never change a command's behavior.
    pub fn record_command(category: &str, ok: bool, notify: bool) {
        if !ALLOWED_COMMANDS.contains(&category) {
            return;
        }
        if disposition() != Disposition::Enabled {
            return;
        }
        let dir = state_dir();
        let _ = std::fs::create_dir_all(&dir);
        accumulate(&dir, category, ok);

        // Notice before first send (ADR-026): show it once; send nothing
        // on the day it is shown.
        if !dir.join("notice-shown").exists() {
            show_notice();
            let _ = std::fs::write(dir.join("notice-shown"), b"1");
            return;
        }

        let today = today_utc();
        let last = std::fs::read_to_string(dir.join("last-check")).unwrap_or_default();
        if last.trim() == today {
            return;
        }
        // Mark the day attempted BEFORE sending: a failed send must not turn
        // into more attempts (no retries, ever).
        let _ = std::fs::write(dir.join("last-check"), &today);

        let payload = drain_payload(&dir);
        if let Some(latest) = send(&payload)
            && notify
        {
            maybe_print_update_notice(&dir, &latest);
        }
    }

    /// Add one outcome to the pending counter file.
    fn accumulate(dir: &Path, category: &str, ok: bool) {
        let path = dir.join("pending.json");
        let mut counts: BTreeMap<String, super::imp::CategoryCount> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let entry = counts.entry(category.to_string()).or_default();
        if ok {
            entry.ok += 1;
        } else {
            entry.err += 1;
        }
        if let Ok(bytes) = serde_json::to_vec(&counts) {
            let _ = std::fs::write(&path, bytes);
        }
    }

    /// Build the payload from pending counters and reset them.
    fn drain_payload(dir: &Path) -> Payload {
        let path = dir.join("pending.json");
        let counts: BTreeMap<String, CategoryCount> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        Payload {
            schema: 1,
            id: anon_id(dir),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            dist: option_env!("GANG_DIST").unwrap_or("source").to_string(),
            counts,
        }
    }

    /// The payload that WOULD be sent right now (for `gang telemetry show`),
    /// without draining the counters or sending anything.
    pub fn peek_payload() -> Payload {
        let dir = state_dir();
        let counts: BTreeMap<String, CategoryCount> = std::fs::read(dir.join("pending.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Payload {
            schema: 1,
            id: anon_id(&dir),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            dist: option_env!("GANG_DIST").unwrap_or("source").to_string(),
            counts,
        }
    }

    /// Random, resettable, never derived from anything (uuid v4 from the same
    /// RNG used elsewhere in the workspace).
    fn anon_id(dir: &Path) -> String {
        let path = dir.join("id");
        if let Ok(existing) = std::fs::read_to_string(&path) {
            let trimmed = existing.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, &id);
        id
    }

    /// One POST, two-second budget, no retries. Returns the server-reported
    /// latest version on success; `None` on ANY failure (all failures are
    /// equivalent and silent).
    fn send(payload: &Payload) -> Option<String> {
        let body = serde_json::to_vec(payload).ok()?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(SEND_TIMEOUT))
            .max_redirects(0)
            .build()
            .into();
        let mut response = agent
            .post(ENDPOINT)
            .header("content-type", "application/json")
            .send(&body[..])
            .ok()?;
        let text = response
            .body_mut()
            .with_config()
            .limit(4096)
            .read_to_string()
            .ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        value
            .get("latest")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// Print the update notice at most once per new version, to stderr.
    fn maybe_print_update_notice(dir: &Path, latest: &str) {
        let current = env!("CARGO_PKG_VERSION");
        if !version_is_newer(latest, current) {
            return;
        }
        let marker = dir.join("last-notified-version");
        if std::fs::read_to_string(&marker).is_ok_and(|v| v.trim() == latest) {
            return;
        }
        let _ = std::fs::write(&marker, latest);
        eprintln!(
            "gang {latest} is available (you have {current}). Changelog: https://github.com/RobotDen/ganglion/releases"
        );
    }

    /// Numeric semver comparison on the MAJOR.MINOR.PATCH prefix; anything
    /// unparseable is "not newer" (never nag on garbage).
    fn version_is_newer(candidate: &str, current: &str) -> bool {
        fn parts(v: &str) -> Option<[u64; 3]> {
            let core = v.split(['-', '+']).next()?;
            let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
            Some([it.next()??, it.next()??, it.next()??])
        }
        match (parts(candidate), parts(current)) {
            (Some(c), Some(n)) => c > n,
            _ => false,
        }
    }

    fn today_utc() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }

    /// The first-run disclosure (verbatim copy lives in TELEMETRY.md; the
    /// two must not drift — a doc test asserts containment).
    pub const NOTICE: &str = "\
Ganglion telemetry notice (shown once)
--------------------------------------
To help us build better tools, the gang CLI sends ONE anonymous request per
day from operator commands: a random id (no machine or identity data), the
CLI version, OS/arch, install channel, and per-command success/error counts.
Never arguments, names, patterns, peers, URLs, or anything from your network.
Inspect exactly what would be sent:   gang telemetry show
Disable with any of:                  gang telemetry off
                                      export DO_NOT_TRACK=1
The full story, field list, and every opt-out: TELEMETRY.md in the repo.

  Telemetry never runs from `gang agent`, `gang join`, or `gang relay` —
  but if you operate Ganglion in production, on robots or in customer
  environments, DISABLE IT OUTRIGHT: `gang telemetry off` on every
  operator workstation. Nothing was sent today; sending starts tomorrow.";

    fn show_notice() {
        eprintln!("{NOTICE}");
    }

    /// `gang telemetry <status|show|on|off|reset>`.
    pub fn telemetry_cli(action: &TelemetryAction) -> anyhow::Result<()> {
        let dir = state_dir();
        match action {
            TelemetryAction::Status => {
                match disposition() {
                    Disposition::Enabled => {
                        println!("Telemetry: ENABLED (anonymous, one request per day maximum).");
                    }
                    Disposition::DisabledBy(layer) => {
                        println!("Telemetry: DISABLED by {layer}.");
                    }
                }
                let id = std::fs::read_to_string(dir.join("id"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "(not yet generated)".into());
                let last = std::fs::read_to_string(dir.join("last-check"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "(never)".into());
                println!("Anonymous id: {id}");
                println!("Last checkpoint day: {last}");
                println!("Endpoint: {ENDPOINT}");
                println!("Details and all opt-outs: TELEMETRY.md");
            }
            TelemetryAction::Show => {
                let payload = peek_payload();
                println!("{}", serde_json::to_string_pretty(&payload)?);
                eprintln!("\n(This is byte-for-byte what the next daily checkpoint would send.)");
            }
            TelemetryAction::On => {
                set_config_enabled(true)?;
                println!("Telemetry enabled in config.toml. (Environment opt-outs still win.)");
            }
            TelemetryAction::Off => {
                set_config_enabled(false)?;
                println!(
                    "Telemetry disabled in config.toml. Nothing will be sent from this \
                     machine. (For production fleets, set this on every operator \
                     workstation — see TELEMETRY.md.)"
                );
            }
            TelemetryAction::Reset => {
                let _ = std::fs::remove_file(dir.join("id"));
                let _ = std::fs::remove_file(dir.join("pending.json"));
                println!("Anonymous id and pending counters reset.");
            }
        }
        Ok(())
    }

    /// Persist `[telemetry] enabled = <value>` into config.toml, preserving
    /// everything else in the file.
    fn set_config_enabled(enabled: bool) -> anyhow::Result<()> {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        let mut value: toml::Value = match std::fs::read_to_string(&path) {
            Ok(text) => text
                .parse()
                .unwrap_or(toml::Value::Table(Default::default())),
            Err(_) => toml::Value::Table(Default::default()),
        };
        let table = value
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config.toml is not a TOML table"))?;
        let telemetry = table
            .entry("telemetry")
            .or_insert(toml::Value::Table(Default::default()));
        telemetry
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[telemetry] is not a table"))?
            .insert("enabled".into(), toml::Value::Boolean(enabled));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(&value)?)?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn allowlist_excludes_robot_and_field_commands() {
            // The prime constraint, as a test: the processes that run on
            // robots/infrastructure and the field-triage commands are not
            // allowlisted.
            for cmd in [
                "agent",
                "join",
                "relay",
                "doctor",
                "diagnose",
                "test-archetype",
            ] {
                assert!(
                    !ALLOWED_COMMANDS.contains(&cmd),
                    "'{cmd}' must never record telemetry"
                );
            }
        }

        #[test]
        fn accumulate_and_drain_roundtrip() {
            let dir = tempfile::TempDir::new().unwrap();
            accumulate(dir.path(), "deploy", true);
            accumulate(dir.path(), "deploy", true);
            accumulate(dir.path(), "deploy", false);
            accumulate(dir.path(), "policy", true);
            let payload = drain_payload(dir.path());
            assert_eq!(payload.schema, 1);
            assert_eq!(payload.counts["deploy"].ok, 2);
            assert_eq!(payload.counts["deploy"].err, 1);
            assert_eq!(payload.counts["policy"].ok, 1);
            // Drained: a second drain is empty.
            assert!(drain_payload(dir.path()).counts.is_empty());
        }

        #[test]
        fn payload_fields_are_exactly_the_documented_set() {
            let dir = tempfile::TempDir::new().unwrap();
            let payload = drain_payload(dir.path());
            let value = serde_json::to_value(&payload).unwrap();
            let mut keys: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(|s| s.as_str())
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                vec!["arch", "counts", "dist", "id", "os", "schema", "version"],
                "payload gained a field — amend ADR-026 + TELEMETRY.md first"
            );
        }

        #[test]
        fn anon_id_is_stable_and_resettable() {
            let dir = tempfile::TempDir::new().unwrap();
            let a = anon_id(dir.path());
            let b = anon_id(dir.path());
            assert_eq!(a, b, "id must be stable across calls");
            assert_eq!(a.len(), 36, "uuid v4 shape");
            std::fs::remove_file(dir.path().join("id")).unwrap();
            let c = anon_id(dir.path());
            assert_ne!(a, c, "reset must produce a fresh id");
        }

        #[test]
        fn version_comparison_is_numeric_and_garbage_safe() {
            assert!(version_is_newer("2.10.0", "2.9.9"));
            assert!(!version_is_newer("2.5.0", "2.5.0"));
            assert!(!version_is_newer("2.4.9", "2.5.0"));
            assert!(version_is_newer("3.0.0-rc1", "2.9.0"));
            assert!(!version_is_newer("latest", "2.5.0"));
            assert!(!version_is_newer("", "2.5.0"));
        }

        #[test]
        fn notice_matches_telemetry_md_verbatim() {
            // TELEMETRY.md promises its copy of the notice is what the
            // binary prints; this keeps the two from drifting.
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../TELEMETRY.md");
            if let Ok(doc) = std::fs::read_to_string(path) {
                assert!(
                    doc.contains(NOTICE),
                    "TELEMETRY.md no longer contains the notice verbatim"
                );
            }
        }

        #[test]
        fn notice_contains_the_production_warning() {
            assert!(NOTICE.contains("DISABLE IT OUTRIGHT"));
            assert!(NOTICE.contains("gang telemetry off"));
            assert!(NOTICE.contains("DO_NOT_TRACK"));
            assert!(NOTICE.contains("Nothing was sent today"));
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    /// ADR-026 layer 2, as a tripwire: the non-CLI crates must contain zero
    /// references to the telemetry module or its endpoint. (The word
    /// "telemetry" alone is allowed in prose comments — bandwidth.rs uses it
    /// descriptively — so this greps for the load-bearing tokens.)
    #[test]
    fn no_telemetry_references_outside_gang_cli() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for krate in ["gang-core", "gang-ros", "gang-libp2p", "gang-wasm-host"] {
            let src = root.join(krate).join("src");
            if !src.exists() {
                continue; // packaged build outside the workspace
            }
            let mut stack = vec![src];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if path.extension().is_none_or(|e| e != "rs") {
                        continue;
                    }
                    let text = std::fs::read_to_string(&path).unwrap();
                    for token in ["checkpoint.robotden.dev", "mod telemetry", "telemetry::"] {
                        assert!(
                            !text.contains(token),
                            "{} contains '{token}' — telemetry code must live in \
                             gang-cli only (ADR-026)",
                            path.display()
                        );
                    }
                }
            }
        }
    }
}
