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
#[cfg_attr(not(feature = "telemetry"), allow(dead_code))]
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
pub use imp::{fleet_merge_bundle, record_command, telemetry_cli};

#[cfg(not(feature = "telemetry"))]
/// Telemetry compiled out (`--no-default-features`): a no-op.
pub fn record_command(_category: &str, _ok: bool, _notify: bool) {}

#[cfg(not(feature = "telemetry"))]
/// Fleet merge when telemetry is compiled out: refuse loudly — a pull that
/// silently discarded the fetched bundle would be worse than an error.
pub fn fleet_merge_bundle(_peer_id: &str, _bundle_json: &str) -> anyhow::Result<()> {
    anyhow::bail!(
        "telemetry is compiled out of this binary (built without the `telemetry` \
         cargo feature); there is no fleet accumulator to merge into"
    )
}

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
        // NB: a TOML *document* parses to `toml::Table`, not `toml::Value`
        // (Value::from_str rejects table content in toml 1.x).
        let Ok(table) = text.parse::<toml::Table>() else {
            return false;
        };
        table
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
        let latest = send(&payload);

        // ADR-027: fleet forwarding rides the same daily flush, and ONLY
        // behind its own explicit opt-in (`gang telemetry fleet on`) on top
        // of the disposition gate already passed above. Off by default.
        if config_fleet_enabled()
            && let Some(fleet) = drain_fleet_payload(&dir)
        {
            send_fleet(&fleet);
        }

        if let Some(latest) = latest
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
            TelemetryAction::Fleet { action } => return fleet_cli(action),
        }
        Ok(())
    }

    /// `gang telemetry fleet <status|on|off|show|reset>`. (`pull` is async
    /// and dispatched from `commands::fleet_pull`.)
    fn fleet_cli(action: &crate::FleetAction) -> anyhow::Result<()> {
        use crate::FleetAction;
        match action {
            FleetAction::Status => {
                let forwarding = match (disposition(), config_fleet_enabled()) {
                    (Disposition::Enabled, true) => "ON".to_string(),
                    (Disposition::Enabled, false) => {
                        "OFF (default — enable with `gang telemetry fleet on`)".to_string()
                    }
                    (Disposition::DisabledBy(layer), true) => {
                        format!("OFF (opted in, but telemetry is disabled by {layer})")
                    }
                    (Disposition::DisabledBy(layer), false) => {
                        format!("OFF (and telemetry is disabled by {layer})")
                    }
                };
                println!("Fleet forwarding: {forwarding}");
                let state = load_fleet_state();
                println!(
                    "Pulled since last flush: {} robot(s), {} agent version(s)",
                    state.pulled.len(),
                    state.agent_versions.len()
                );
                println!("Robot bundles are local-only until pulled; see TELEMETRY.md.");
            }
            FleetAction::On => {
                set_config_fleet(true)?;
                println!(
                    "Fleet forwarding enabled in config.toml: pulled robot bundles will be \
                     aggregated and included (bucketed robot count, summed counts, no \
                     per-robot rows) in the daily checkpoint. `gang telemetry fleet show` \
                     previews the exact payload. Environment opt-outs still win."
                );
            }
            FleetAction::Off => {
                set_config_fleet(false)?;
                println!(
                    "Fleet forwarding disabled in config.toml (the default). Pulled \
                     bundles stay on this machine."
                );
            }
            FleetAction::Show => match peek_fleet_payload() {
                Some(payload) => {
                    println!("{}", serde_json::to_string_pretty(&payload)?);
                    eprintln!(
                        "\n(This is byte-for-byte what the next daily flush would send to \
                         /v1/fleet — and only if fleet forwarding is on.)"
                    );
                }
                None => println!(
                    "Nothing pulled yet — `gang telemetry fleet pull <robot>` fetches \
                     robot usage bundles into the local accumulator."
                ),
            },
            FleetAction::Reset => {
                let _ = std::fs::remove_file(fleet_state_path());
                println!("Local fleet accumulator cleared.");
            }
            FleetAction::Pull { .. } => unreachable!("pull is dispatched in commands::fleet_pull"),
        }
        Ok(())
    }

    /// Persist `[telemetry] enabled = <value>` into config.toml, preserving
    /// everything else in the file.
    fn set_config_enabled(enabled: bool) -> anyhow::Result<()> {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        // Parse the existing DOCUMENT as a Table so every other setting in
        // config.toml is preserved. A malformed file is an error, never a
        // silent overwrite of the user's configuration.
        let mut table: toml::Table = match std::fs::read_to_string(&path) {
            Ok(text) => text.parse().map_err(|e| {
                anyhow::anyhow!("config.toml is not valid TOML ({e}); refusing to rewrite it")
            })?,
            Err(_) => toml::Table::default(),
        };
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
        std::fs::write(&path, toml::to_string_pretty(&table)?)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Fleet telemetry (ADR-027): operator-side accumulator + opt-in
    // forwarding of robot usage bundles pulled with
    // `gang telemetry fleet pull`.
    // ------------------------------------------------------------------

    /// Fleet endpoint (same worker, published in-repo like the checkpoint).
    const FLEET_ENDPOINT: &str = "https://checkpoint.robotden.dev/v1/fleet";

    /// The COMPLETE fleet payload (ADR-027: exhaustive; adding a field
    /// requires amending the ADR and TELEMETRY.md, and bumping `schema`).
    #[derive(Debug, Serialize, Deserialize)]
    pub struct FleetPayload {
        /// Payload schema version.
        pub schema: u32,
        /// The operator's ADR-026 anonymous id (random, resettable).
        pub id: String,
        /// Operator CLI version.
        pub version: String,
        /// Bucketed distinct-robot count: "1" | "2-5" | "6-20" | "21-100"
        /// | "100+" — bucketed so small fleets aren't fingerprintable.
        pub robots: String,
        /// Unique agent versions seen across pulled bundles, sorted. No
        /// per-version robot counts, for the same fingerprinting reason.
        pub agent_versions: Vec<String>,
        /// Capability-*group* ok/err counts, summed across the whole fleet
        /// before sending. Per-robot rows never leave this machine.
        pub counts: BTreeMap<String, CategoryCount>,
        /// Per-category failure-kind breakouts, summed across the fleet.
        /// Kinds are the robot runtime's CLOSED set ("trapped", "deadline",
        /// "policy-denied", "fuel-exhausted", "hash-mismatch", "failed").
        pub errors: BTreeMap<String, BTreeMap<String, u64>>,
        /// Total policy denials across the fleet (a bare count).
        pub denials: u64,
    }

    /// LOCAL fleet accumulator. `pulled` holds peer ids so distinct robots
    /// are counted correctly across multiple pulls — that list is the
    /// operator's own data (same as peers.json) and is reduced to the
    /// bucketed `robots` string before anything is sent.
    #[derive(Debug, Default, Serialize, Deserialize)]
    struct FleetState {
        pulled: std::collections::BTreeSet<String>,
        agent_versions: std::collections::BTreeSet<String>,
        counts: BTreeMap<String, CategoryCount>,
        errors: BTreeMap<String, BTreeMap<String, u64>>,
        denials: u64,
    }

    /// The robot bundle as fetched (mirrors `gang-ros::usage_bundle`).
    /// Unknown fields are rejected: a drifted future agent must not smuggle
    /// new data through an old operator CLI unvalidated.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RobotBundle {
        schema: u32,
        version: String,
        #[allow(dead_code)]
        os: String,
        #[allow(dead_code)]
        arch: String,
        counts: BTreeMap<String, CategoryCount>,
        /// Absent on bundles from agents predating error breakouts.
        #[serde(default)]
        errors: BTreeMap<String, BTreeMap<String, u64>>,
        denials: u64,
    }

    fn fleet_state_path() -> PathBuf {
        state_dir().join("fleet.json")
    }

    fn load_fleet_state() -> FleetState {
        std::fs::read(fleet_state_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn store_fleet_state(state: &FleetState) {
        let dir = state_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(bytes) = serde_json::to_vec(state) {
            let _ = std::fs::write(fleet_state_path(), bytes);
        }
    }

    /// `[telemetry] fleet = true` in config.toml — the explicit forwarding
    /// opt-in (`gang telemetry fleet on`). Default: absent = off.
    fn config_fleet_enabled() -> bool {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(table) = text.parse::<toml::Table>() else {
            return false;
        };
        table
            .get("telemetry")
            .and_then(|t| t.get("fleet"))
            .and_then(|e| e.as_bool())
            == Some(true)
    }

    /// Persist `[telemetry] fleet = <value>` into config.toml, preserving
    /// everything else (same discipline as [`set_config_enabled`]).
    fn set_config_fleet(enabled: bool) -> anyhow::Result<()> {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        let mut table: toml::Table = match std::fs::read_to_string(&path) {
            Ok(text) => text.parse().map_err(|e| {
                anyhow::anyhow!("config.toml is not valid TOML ({e}); refusing to rewrite it")
            })?,
            Err(_) => toml::Table::default(),
        };
        let telemetry = table
            .entry("telemetry")
            .or_insert(toml::Value::Table(Default::default()));
        telemetry
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[telemetry] is not a table"))?
            .insert("fleet".into(), toml::Value::Boolean(enabled));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(&table)?)?;
        Ok(())
    }

    /// Merge one fetched robot bundle into the local fleet accumulator.
    /// Validates the bundle strictly (schema 1, known fields only) — a
    /// bundle that fails validation is discarded with an error rather than
    /// merged. The peer id is used only for local distinct-robot counting.
    pub fn fleet_merge_bundle(peer_id: &str, bundle_json: &str) -> anyhow::Result<()> {
        let bundle: RobotBundle = serde_json::from_str(bundle_json)
            .map_err(|e| anyhow::anyhow!("robot bundle failed validation, not merged: {e}"))?;
        if bundle.schema != 1 {
            anyhow::bail!(
                "robot bundle schema {} is not supported by this CLI, not merged",
                bundle.schema
            );
        }
        let mut state = load_fleet_state();
        state.pulled.insert(peer_id.to_string());
        state.agent_versions.insert(bundle.version);
        for (category, pair) in bundle.counts {
            let entry = state.counts.entry(category).or_default();
            entry.ok += pair.ok;
            entry.err += pair.err;
        }
        for (category, kinds) in bundle.errors {
            let entry = state.errors.entry(category).or_default();
            for (kind, n) in kinds {
                *entry.entry(kind).or_default() += n;
            }
        }
        state.denials += bundle.denials;
        store_fleet_state(&state);
        Ok(())
    }

    fn robots_bucket(n: usize) -> &'static str {
        match n {
            0..=1 => "1",
            2..=5 => "2-5",
            6..=20 => "6-20",
            21..=100 => "21-100",
            _ => "100+",
        }
    }

    fn state_to_payload(state: &FleetState, dir: &Path) -> FleetPayload {
        FleetPayload {
            schema: 1,
            id: anon_id(dir),
            version: env!("CARGO_PKG_VERSION").to_string(),
            robots: robots_bucket(state.pulled.len()).to_string(),
            agent_versions: state.agent_versions.iter().cloned().collect(),
            counts: state.counts.clone(),
            errors: state.errors.clone(),
            denials: state.denials,
        }
    }

    /// The fleet payload that WOULD be sent (for `gang telemetry fleet
    /// show`); `None` when nothing has been pulled since the last flush.
    pub fn peek_fleet_payload() -> Option<FleetPayload> {
        let state = load_fleet_state();
        if state.pulled.is_empty() {
            return None;
        }
        Some(state_to_payload(&state, &state_dir()))
    }

    /// Build the fleet payload and reset the accumulator (checkpoint
    /// semantics: flushed whether or not the send succeeds; no retries).
    fn drain_fleet_payload(dir: &Path) -> Option<FleetPayload> {
        let state = load_fleet_state();
        if state.pulled.is_empty() {
            return None;
        }
        let _ = std::fs::remove_file(fleet_state_path());
        Some(state_to_payload(&state, dir))
    }

    /// One POST to the fleet endpoint. Same discipline as [`send`]: 2s
    /// budget, no retries, every failure silent and equivalent.
    fn send_fleet(payload: &FleetPayload) {
        let Ok(body) = serde_json::to_vec(payload) else {
            return;
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(SEND_TIMEOUT))
            .max_redirects(0)
            .build()
            .into();
        let _ = agent
            .post(FLEET_ENDPOINT)
            .header("content-type", "application/json")
            .send(&body[..]);
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

        /// ADR-027: the fleet payload field list is exhaustive too.
        #[test]
        fn fleet_payload_field_list_is_locked() {
            let payload = FleetPayload {
                schema: 1,
                id: "x".into(),
                version: "0.0.0".into(),
                robots: "1".into(),
                agent_versions: vec![],
                counts: BTreeMap::new(),
                errors: BTreeMap::new(),
                denials: 0,
            };
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
                vec![
                    "agent_versions",
                    "counts",
                    "denials",
                    "errors",
                    "id",
                    "robots",
                    "schema",
                    "version"
                ],
                "fleet payload gained a field — amend ADR-027 + TELEMETRY.md first"
            );
        }

        /// ADR-027: bucketed robot counts, never exact ones.
        #[test]
        fn robots_bucket_boundaries() {
            for (n, want) in [
                (0, "1"),
                (1, "1"),
                (2, "2-5"),
                (5, "2-5"),
                (6, "6-20"),
                (20, "6-20"),
                (21, "21-100"),
                (100, "21-100"),
                (101, "100+"),
                (5000, "100+"),
            ] {
                assert_eq!(robots_bucket(n), want, "bucket({n})");
            }
        }

        /// ADR-027: a robot bundle that drifts (unknown fields, wrong
        /// schema) is rejected at merge, never forwarded unvalidated.
        #[test]
        fn robot_bundle_validation_is_strict() {
            let good = r#"{"schema":1,"version":"2.6.0","os":"linux","arch":"x86_64",
                "counts":{"ros":{"ok":1,"err":0}},"denials":0}"#;
            assert!(serde_json::from_str::<RobotBundle>(good).is_ok());

            let unknown_field = r#"{"schema":1,"version":"2.6.0","os":"linux","arch":"x86_64",
                "counts":{},"denials":0,"robot_name":"acme-line3"}"#;
            assert!(
                serde_json::from_str::<RobotBundle>(unknown_field).is_err(),
                "unknown fields must be rejected, not silently forwarded"
            );

            let missing = r#"{"schema":1,"version":"2.6.0"}"#;
            assert!(serde_json::from_str::<RobotBundle>(missing).is_err());
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
        fn config_document_parses_as_table_and_off_preserves_other_settings() {
            // Regression: toml::Value rejects document content in toml 1.x —
            // the config check must parse a Table, and `telemetry off` must
            // never clobber unrelated settings.
            let text = "default_relay = \"/dns4/r/tcp/443\"\n\n[telemetry]\nenabled = false\n";
            let table: toml::Table = text.parse().unwrap();
            assert_eq!(
                table
                    .get("telemetry")
                    .and_then(|t| t.get("enabled"))
                    .and_then(|e| e.as_bool()),
                Some(false)
            );
            // Round-trip preserves the unrelated key.
            let mut table = table;
            table
                .get_mut("telemetry")
                .and_then(|t| t.as_table_mut())
                .unwrap()
                .insert("enabled".into(), toml::Value::Boolean(true));
            let out = toml::to_string_pretty(&table).unwrap();
            assert!(out.contains("default_relay"));
            assert!(out.contains("enabled = true"));
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

    /// ADR-027: the robot-side usage bundle is local-only by construction.
    /// Its module must never grow a network client or endpoint reference —
    /// the *transmission* boundary is the prime constraint, and this
    /// tripwire keeps the send path structurally impossible on robots.
    #[test]
    fn usage_bundle_module_has_no_network_code() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gang-ros/src/usage_bundle.rs");
        if !path.exists() {
            return; // packaged build outside the workspace
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for token in [
            "ureq",
            "reqwest",
            "hyper",
            "TcpStream",
            "UdpSocket",
            "robotden.dev",
            "http://",
            "https://",
        ] {
            assert!(
                !text.contains(token),
                "usage_bundle.rs contains '{token}' — the robot bundle must have no \
                 send path (ADR-027)"
            );
        }
    }
}
