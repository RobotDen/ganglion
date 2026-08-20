mod commands;
mod doctor;
mod fleet_html;
mod foxglove;
mod link_profile;
mod mcp;
mod telemetry;
mod tui;

use clap::{CommandFactory, Parser, Subcommand};

/// gang — reach, observe, and act on ROS 2 robots behind hostile networks.
///
/// Ganglion provides the connectivity and tool-execution substrate for fleet
/// operators to reach robots deployed inside networks they don't own.
#[derive(Parser)]
#[command(
    name = "gang",
    version,
    about,
    after_help = "Run 'gang demo' for a self-contained end-to-end demo. Docs: docs/QUICKSTART.md"
)]
struct Cli {
    /// Output format: "text" (default) or "json" for structured output.
    #[arg(long, default_value = "text", global = true)]
    format: OutputFormat,

    /// Increase log verbosity: -v = debug, -vv = trace (gang crates),
    /// -vvv = trace (all crates). Ignored when RUST_LOG is set.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress info/warn logs (errors only). Ignored when RUST_LOG is set.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Point the whole CLI at a self-contained fleet directory instead of
    /// `~/.gang` (identity, peer registry, config, trust). This is the dir
    /// `gang up` stands a local fleet up in; pass the same value here to drive
    /// that fleet: `gang --data-dir <dir> deploy up-robot …`.
    #[arg(long, value_name = "PATH", global = true)]
    data_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Event-feed transport selector for `logs`/`connect`/`tui` (ADR-024). Maps to
/// [`gang_libp2p::EventsTransport`]; `auto` prefers push and falls back to poll.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum EventsTransportArg {
    /// Prefer the push substream; fall back to poll automatically (default).
    Auto,
    /// Force the push substream; error if it cannot be opened.
    Push,
    /// Force the request-response poll; never open a push stream.
    Poll,
}

impl From<EventsTransportArg> for gang_libp2p::EventsTransport {
    fn from(a: EventsTransportArg) -> Self {
        match a {
            EventsTransportArg::Auto => Self::Auto,
            EventsTransportArg::Push => Self::Push,
            EventsTransportArg::Poll => Self::Poll,
        }
    }
}

/// Reject `--format json` on subcommands that only produce human-readable
/// output, rather than silently emitting text when JSON was requested.
fn reject_json(format: &OutputFormat, command: &str) -> anyhow::Result<()> {
    if matches!(format, OutputFormat::Json) {
        anyhow::bail!(
            "`gang {command}` does not support `--format json`; omit it for text output."
        );
    }
    Ok(())
}

// Subcommands are ordered into logical groups via `display_order` (clap 4 has
// no help_heading for subcommands): Identity & Trust (10s), Capabilities
// (20s), Network (30s), Registry & Artifacts (40s), Diagnostics (50s).
#[derive(Subcommand)]
enum Commands {
    /// Guided first-run setup — go from installed to configured in one command.
    ///
    /// Detects the network archetype (like `gang diagnose`), generates the
    /// operator identity if none exists, writes a default-deny `policy.toml`
    /// with commented example rules plus an operator `config.toml`, and prints
    /// exactly what to run next. Interactive on a TTY; runs non-interactively
    /// with safe defaults when stdin is a pipe or `--yes` is given. Existing
    /// files are never clobbered without `--force`.
    #[command(display_order = 9)]
    Init {
        /// Overwrite an existing identity, policy, or config.
        #[arg(long)]
        force: bool,
        /// Skip prompts and use safe defaults (implied when stdin is not a
        /// TTY, e.g. in CI or a pipe).
        #[arg(long, short = 'y', visible_alias = "non-interactive")]
        yes: bool,
        /// Emit the resulting setup (identity id, archetype, paths) as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Enroll a robot with one copy-paste line — the `gang up` for robots.
    ///
    /// Run on the OPERATOR machine. Mints a short-lived, single-use pairing
    /// token bound to the relay and this operator's identity, prints ONE line to
    /// run on the robot (`gang join gang1_…`), then waits: when the robot dials
    /// out and enrolls, the operator records it — under the identity libp2p
    /// authenticated on the wire, never a self-report — and it appears in
    /// `gang peer list`, ready for `gang deploy`/`gang run`. Needs a relay
    /// (`--relay`, else `default_relay` from config).
    #[command(display_order = 8)]
    Pair {
        /// Relay multiaddr the robot should dial (default: `default_relay`).
        #[arg(long, short = 'r', value_name = "MULTIADDR")]
        relay: Option<String>,
        /// Name to register the paired robot under (default: `robot-<short-id>`).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Token lifetime, e.g. `15m`, `1h`, `90s` (default: 15m).
        #[arg(long, value_name = "DURATION")]
        expires: Option<String>,
        /// Also render the robot line as a QR code (when supported).
        #[arg(long)]
        qr: bool,
        /// Give up waiting for the robot after this many seconds (default: 300).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
        /// Emit the token/relay/operator facts as JSON, then wait as usual.
        #[arg(long)]
        json: bool,
    },

    /// Join a fleet from a pairing token — the ONE line you run on the robot.
    ///
    /// Run on the ROBOT. Decodes the `gang join gang1_…` token from `gang pair`,
    /// loads or generates this robot's identity, dials out to the relay, reserves
    /// a circuit, and enrolls with the operator the token names (whose identity
    /// libp2p authenticates end-to-end). Then it keeps serving as the agent so
    /// the operator can deploy — exactly like `gang agent`. Pass `--once` to
    /// enroll and exit instead of staying online.
    #[command(display_order = 9)]
    Join {
        /// The pairing token printed by `gang pair` (`gang1_…`).
        token: String,
        /// Name to request from the operator (default: `robot-<short-id>`).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Enroll and exit instead of staying online as the agent.
        #[arg(long)]
        once: bool,
        /// Overall timeout in seconds for the enrollment exchange (default: 60).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
        /// Emit the enrollment result as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Manage peer identity.
    #[command(display_order = 10, visible_alias = "id")]
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },

    /// Sign a WASM component with your identity key.
    #[command(display_order = 11)]
    Sign {
        /// Path to the .wasm component to sign.
        wasm_path: String,
        /// Path to the signing key (default: ~/.gang/identity.key).
        #[arg(long)]
        key: Option<String>,
        /// Component name (default: derived from filename).
        #[arg(long)]
        name: Option<String>,
        /// Component version (semver). Distinct from the CLI's own -V/--version.
        #[arg(
            long = "component-version",
            visible_alias = "version",
            value_name = "SEMVER",
            default_value = "0.1.0"
        )]
        version: String,
        /// Declared capability groups, comma-separated (e.g.
        /// "diagnostics,logs,ros,fs,artifacts,process,network,metrics,http").
        /// If omitted, a permissive default set is used with a warning.
        #[arg(long, value_delimiter = ',')]
        capabilities: Option<Vec<String>>,
        /// Declare an allowed HTTP endpoint URL pattern (repeatable; implies
        /// the http group). Read-only (GET/HEAD) unless suffixed `:rw`, e.g.
        /// `--http-endpoint "https://api.example.com/v1/**:rw"`. (ADR-025)
        #[arg(long = "http-endpoint", value_name = "PATTERN[:rw]")]
        http_endpoints: Vec<String>,
        /// Credential slot names this component consumes, comma-separated
        /// (#43). Bound to secret files robot-side in credentials.toml.
        #[arg(long = "credential-slots", value_delimiter = ',')]
        credential_slots: Vec<String>,
        /// Exported entry points beyond the default `run`, comma-separated
        /// (#42). Declarative, for pre-flight visibility.
        #[arg(long, value_delimiter = ',')]
        exports: Vec<String>,
    },

    /// Run the robot agent (for development/testing).
    /// Starts a local Ganglion agent that listens for operator connections.
    #[command(display_order = 30)]
    Agent {
        /// Path to the agent config file.
        #[arg(long)]
        config: Option<String>,
        /// Directory for capabilities and state (default: /tmp/gang-agent).
        #[arg(long, default_value = "/tmp/gang-agent")]
        data_dir: String,
        /// Relay multiaddr to dial for remote connectivity.
        /// Without this flag, the agent runs in local-only mode.
        #[arg(long, short = 'r')]
        relay: Option<String>,
    },

    /// Deploy a signed capability to a robot.
    #[command(display_order = 21)]
    Deploy {
        /// Robot name, abbreviated peer ID, or full peer ID.
        robot: String,
        /// Path to the signed .wasm component.
        wasm_path: String,
        /// Path to the manifest (.manifest.cbor), auto-detected if adjacent.
        #[arg(long)]
        manifest: Option<String>,
        /// Explicit peer ID (bypasses name/prefix resolution).
        #[arg(long, short = 'p')]
        peer: Option<String>,
        /// Relay multiaddr (overrides registry and config defaults).
        #[arg(long, short = 'r')]
        relay: Option<String>,
        /// Overall timeout in seconds for a remote deploy (default: 60).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
    },

    /// Invoke an installed capability on a robot.
    #[command(display_order = 22)]
    Run {
        /// Robot name, abbreviated peer ID, or full peer ID.
        robot: String,
        /// Capability name to invoke.
        cap_name: String,
        /// Arguments to pass to the capability.
        args: Vec<String>,
        /// Explicit peer ID (bypasses name/prefix resolution).
        #[arg(long, short = 'p')]
        peer: Option<String>,
        /// Relay multiaddr (overrides registry and config defaults).
        #[arg(long, short = 'r')]
        relay: Option<String>,
        /// Overall timeout in seconds for a remote invocation (default: 30).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
        /// Invoke this exported function instead of the default `run` (#42).
        /// The export must have the standard capability signature.
        #[arg(long, value_name = "NAME")]
        export: Option<String>,
    },

    /// List capabilities installed on a robot.
    #[command(display_order = 23)]
    Caps {
        /// Robot name, abbreviated peer ID, or full peer ID.
        robot: String,
        /// Explicit peer ID (bypasses name/prefix resolution).
        #[arg(long, short = 'p')]
        peer: Option<String>,
        /// Relay multiaddr (overrides registry and config defaults).
        #[arg(long, short = 'r')]
        relay: Option<String>,
        /// Overall timeout in seconds for a remote listing (default: 30).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
    },

    /// Stream a robot's audit + policy events over the relay circuit.
    #[command(display_order = 34)]
    Logs {
        /// Robot name, abbreviated peer ID, or full peer ID.
        robot: String,
        /// Follow output live (like tail -f); Ctrl-C to stop.
        #[arg(long)]
        follow: bool,
        /// Only show events newer than this (e.g. 30s, 5m, 2h, 1d).
        #[arg(long, value_name = "DUR")]
        since: Option<String>,
        /// Explicit peer ID (bypasses name/prefix resolution).
        #[arg(long, short = 'p')]
        peer: Option<String>,
        /// Relay multiaddr (overrides registry and config defaults).
        #[arg(long, short = 'r')]
        relay: Option<String>,
        /// Event-feed transport: auto (default), push, or poll (ADR-024).
        #[arg(long, value_name = "MODE")]
        events_transport: Option<EventsTransportArg>,
    },

    /// Run a local end-to-end demo: start agent, deploy diagnostics, invoke.
    #[command(display_order = 52)]
    Demo,

    /// Stand up a real local fleet: relay + agent (default-deny) + signed sample.
    ///
    /// The bridge between `gang demo` (self-contained, tears itself down) and a
    /// hand-wired relay/agent/deploy. `gang up` starts a loopback relay and a
    /// robot agent under one working directory, signs one sample capability with
    /// your operator identity, registers the robot, and prints the exact `gang`
    /// commands to drive it from another terminal. Blocks until Ctrl-C, then
    /// tears the fleet down.
    #[command(display_order = 55, visible_alias = "fleet")]
    Up {
        /// Relay TCP port on loopback (default: an ephemeral port).
        #[arg(long, value_name = "PORT")]
        port: Option<u16>,
        /// Reset the data dir if it already exists (fresh keys and state).
        #[arg(long)]
        force: bool,
        /// Emit the fleet facts as JSON (for scripting), then keep serving.
        #[arg(long)]
        json: bool,
    },

    /// Run a local test harness scenario (requires Docker).
    #[command(display_order = 53)]
    TestArchetype {
        /// Archetype: open-warehouse, nat-office, enterprise-dmz, mobile-cgnat.
        archetype: String,
    },

    /// Diagnose network environment — detect archetype and recommend transport config.
    #[command(display_order = 51, visible_alias = "dx")]
    Diagnose {
        /// Robot name or peer ID (optional — if omitted, probes local network).
        robot: Option<String>,
    },

    /// Print exactly what this network permits — the field-engineer's egress check.
    ///
    /// Runs a handful of outbound-reachability probes (TCP 443, UDP/QUIC,
    /// non-443 TCP, DNS) and — if a relay is configured or given with
    /// `--relay` — whether that relay's transport address is reachable. Prints
    /// a PASS/FAIL table plus a copy-pasteable egress allowlist to hand to the
    /// customer's network/security team. Exits non-zero when no viable
    /// outbound path to a relay exists, so it drops straight into a support
    /// thread: "run `gang doctor` and paste the output."
    #[command(display_order = 51)]
    Doctor {
        /// Relay multiaddr to test reachability against (default: `default_relay`).
        #[arg(long, short = 'r', value_name = "MULTIADDR")]
        relay: Option<String>,

        /// Measure the link and write a deterministic degraded-link profile
        /// (test-harness fixture format) to this path — the customer's
        /// network as a replayable `run-matrix.sh` test case. (#33)
        #[arg(long, value_name = "FILE")]
        profile_out: Option<std::path::PathBuf>,

        /// Profile name recorded in the emitted file (sanitized to
        /// `[a-z0-9-]`). Default: "site".
        #[arg(long, value_name = "NAME", default_value = "site")]
        profile_name: String,

        /// Number of RTT/loss probe samples for `--profile-out` (more
        /// samples = finer loss resolution: 40 resolves ~2.5%).
        #[arg(long, value_name = "N", default_value_t = 40)]
        samples: u16,

        /// Robot uplink cap to record in the profile, kbit/s (rates are not
        /// measurable from a handshake probe — supply from a site speed test).
        #[arg(long, value_name = "KBIT")]
        uplink_kbit: Option<u32>,

        /// Robot downlink cap to record in the profile, kbit/s.
        #[arg(long, value_name = "KBIT")]
        downlink_kbit: Option<u32>,
    },

    /// List bandwidth profiles for degraded-link streaming (`--profile <name>`).
    ///
    /// Shows the built-in presets (`full`, `lidar-low`, `vision-low`,
    /// `logs-only`) plus any operator-defined profiles from config. These names
    /// are accepted by streaming surfaces such as `gang view` to trade fidelity
    /// for reachability on cellular / warehouse-Wi-Fi links.
    #[command(display_order = 51)]
    Profiles,

    /// Fire webhooks when a metric breaches a threshold — `gang alert`.
    ///
    /// The useful 20% of alerting: a rule (metric, comparator, threshold,
    /// cooldown) fires a Slack-compatible JSON webhook on breach. Rules and the
    /// default webhook live in `~/.gang/config.toml`. Delivery is a JSON POST
    /// via curl; `--dry-run` prints the payload instead.
    #[command(display_order = 37)]
    Alert {
        #[command(subcommand)]
        action: AlertAction,
    },

    /// Serve Ganglion tools to an AI agent over MCP (stdio) — `gang mcp`.
    ///
    /// A Model Context Protocol server exposing a curated, read-only
    /// fleet-discovery toolset (status, peers, capabilities, `gang doctor`,
    /// bandwidth profiles). The capability sandbox, signed manifests,
    /// default-deny policy, and audit log mean an agent provably cannot exceed
    /// what those mechanisms permit — the safest substrate for AI-generated
    /// tooling. Speaks JSON-RPC 2.0 on stdout; do not combine with `--json`.
    #[command(display_order = 36)]
    Mcp,

    /// Show real per-transport statistics for the live circuit to a robot.
    #[command(display_order = 35)]
    TransportStats {
        /// Robot name, abbreviated peer ID, or full peer ID.
        robot: String,
        /// Explicit peer ID (bypasses name/prefix resolution).
        #[arg(long, short = 'p')]
        peer: Option<String>,
        /// Relay multiaddr (overrides registry and config defaults).
        #[arg(long, short = 'r')]
        relay: Option<String>,
        /// Overall timeout in seconds (default: 30).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
    },

    /// Retrieve an artifact by CID from any reachable peer.
    #[command(display_order = 42)]
    Fetch {
        /// Content identifier of the artifact.
        cid: String,
        /// Output path (default: current directory).
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Publish a local file to the content store and announce its CID.
    #[command(display_order = 41)]
    Push {
        /// Path to the file to publish.
        path: String,
        /// Content type (e.g., "application/octet-stream").
        #[arg(long)]
        content_type: Option<String>,
    },

    /// List locally-stored artifacts.
    #[command(display_order = 43)]
    Artifacts,

    /// Scaffold, inspect, or manage capabilities.
    #[command(display_order = 20, visible_alias = "cap")]
    Capability {
        #[command(subcommand)]
        action: CapabilityAction,
    },

    /// Scaffold a new signed capability — `gang new tool <name>`.
    ///
    /// A guided front door to the author loop: scaffolds the project (manifest,
    /// tests, WIT) and prints the full idea → build → sign → publish path so a
    /// signed tool can go from nothing to the open registry in one sitting.
    #[command(display_order = 20)]
    New {
        #[command(subcommand)]
        what: NewAction,
    },

    /// Manage the capability registry.
    #[command(display_order = 40)]
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },

    /// Manage known peers (robots, relays, operators).
    #[command(display_order = 12)]
    Peer {
        #[command(subcommand)]
        action: PeerAction,
    },

    /// Inspect and evolve the robot's default-deny policy: show it, permit
    /// exactly what a denial asked for, review recent denials, lint for
    /// wide-open drift.
    #[command(display_order = 14)]
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },

    /// View or edit operator configuration (~/.gang/config.toml).
    #[command(display_order = 13)]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Show Ganglion status: version, identity, available and WIP capabilities.
    ///
    /// With `--html <path>`, instead writes a self-contained fleet-status page
    /// (identity, registered peers, capability count, and recent audit) built
    /// entirely from local state — a shareable snapshot, not a live dashboard
    /// (use `gang tui` for live).
    #[command(display_order = 50)]
    Status {
        /// Write a self-contained fleet-status HTML page to this path.
        #[arg(long, value_name = "PATH")]
        html: Option<String>,
    },

    /// List registered robots with live reachability from a presence probe.
    #[command(display_order = 33)]
    List,

    /// Live fleet dashboard: peers, tunnels, policy decisions, audit tail.
    ///
    /// A full-screen ratatui dashboard that subscribes to every registered
    /// robot's event feed and folds it into four live panes — connected peers
    /// (status · transport · RTT), active tunnels (direct/relay · byte
    /// counters), policy allow/deny decisions, and a tailing audit log.
    ///
    /// Keys: ↑↓/j k select a peer · ⏎ inspect it · p pause the feed (for a
    /// clean capture) · / filter · a audit-only fullscreen · ? help · q/Esc
    /// quit. The feed is a genuine server-push substream (ADR-024) — events
    /// land instantly; a live pulse shows it is fresh. Honors NO_COLOR
    /// (monochrome/ASCII) and resizes gracefully.
    #[command(display_order = 32)]
    Tui {
        /// Focus a single registered robot instead of the whole fleet.
        #[arg(long, value_name = "NAME")]
        robot: Option<String>,
        /// Headless snapshot: fold the feed for N poll cycles, print the
        /// rendered frame as text, and exit (no raw terminal — for CI/capture).
        #[arg(long, value_name = "N")]
        frames: Option<usize>,
        /// Run the live dashboard but ignore keyboard input (for unattended
        /// recording); Ctrl-C still quits.
        #[arg(long)]
        no_input: bool,
        /// Event-feed transport: auto (default), push, or poll (ADR-024).
        #[arg(long, value_name = "MODE")]
        events_transport: Option<EventsTransportArg>,
    },

    /// Attach a live status view to a robot (presence + heartbeat + audit tail).
    #[command(display_order = 32)]
    Connect {
        /// Robot name or peer ID.
        robot: String,
        /// Preferred transport order for happy-eyeballs selection.
        #[arg(long, value_delimiter = ',')]
        prefer_transport: Option<Vec<String>>,
        /// Event-feed transport: auto (default), push, or poll (ADR-024).
        #[arg(long, value_name = "MODE")]
        events_transport: Option<EventsTransportArg>,
    },

    /// Bridge a robot's live feed into Foxglove / Lichtblick — `gang view`.
    ///
    /// Opens a local Foxglove WebSocket endpoint (`ws://127.0.0.1:<port>`) and
    /// forwards the robot's live, relay-delivered, capability-scoped Ganglion
    /// event feed as a JSON channel you can watch in the tool you already have
    /// open. `--profile` shapes the stream for degraded links (see
    /// `gang profiles`). `--topics` is reserved for live ROS topic projection.
    #[command(display_order = 32)]
    View {
        /// Robot name or peer ID.
        robot: String,
        /// Local TCP port for the Foxglove WebSocket endpoint (default: 8765).
        #[arg(long, default_value_t = 8765)]
        port: u16,
        /// ROS topics to project (reserved; live topic streaming is pending).
        #[arg(long, value_delimiter = ',', value_name = "TOPIC")]
        topics: Option<Vec<String>>,
        /// Bandwidth profile name for degraded links (see `gang profiles`).
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,
        /// Event-feed transport: auto (default), push, or poll (ADR-024).
        #[arg(long, value_name = "MODE")]
        events_transport: Option<EventsTransportArg>,
    },

    /// Generate shell completion scripts.
    #[command(display_order = 54)]
    Completions {
        /// Shell to generate completions for.
        shell: clap_complete::Shell,
    },

    /// Run a circuit relay v2 server for NAT traversal.
    ///
    /// Starts a Ganglion node in relay-server mode, enabling robot agents
    /// behind NAT to accept inbound connections. This is the bootstrap relay
    /// described in the design spec (relay.gang.tafy.dev).
    #[command(display_order = 31)]
    /// Inspect or control anonymous usage telemetry (see TELEMETRY.md).
    ///
    /// Ganglion's telemetry is operator-side only, anonymous, opt-out, and
    /// sends at most ONE request per day. It never runs from `gang agent`,
    /// `gang join`, or `gang relay`. If you operate Ganglion in production —
    /// on robots or in customer environments — disable it outright with
    /// `gang telemetry off` on every operator workstation.
    #[command(display_order = 60)]
    Telemetry {
        #[command(subcommand)]
        action: TelemetryAction,
    },

    Relay {
        /// Multiaddr(s) to listen on. Can be specified multiple times.
        /// Default: /ip4/0.0.0.0/tcp/4001 and /ip4/0.0.0.0/udp/4001/quic-v1
        #[arg(long, value_name = "ADDR")]
        listen_addr: Option<Vec<String>>,

        /// TCP/UDP port to listen on (shorthand for default addrs on this port).
        #[arg(long, default_value = "4001")]
        port: u16,

        /// Metrics HTTP port (placeholder for future Prometheus endpoint).
        #[arg(long, default_value = "9090")]
        metrics_port: u16,

        /// Directory for the relay's persisted identity key
        /// (uses <DATA_DIR>/identity.key).
        /// Without this, the default ~/.gang/identity.key is used.
        #[arg(long, value_name = "PATH")]
        data_dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum CapabilityAction {
    /// Generate a capability project skeleton.
    Scaffold {
        /// Capability name (e.g., "my-diagnostics").
        name: String,
        /// Language: rust, cpp, python, go.
        #[arg(long, default_value = "rust")]
        language: String,
        /// Output directory (default: current directory).
        #[arg(long)]
        output_dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum AlertAction {
    /// Evaluate configured rules for a metric against a value; fire on breach.
    Check {
        /// Metric name to evaluate (matches `metric` in configured rules).
        metric: String,
        /// Observed value.
        value: f64,
        /// Webhook URL override (default: `alert_webhook` from config).
        #[arg(long, value_name = "URL")]
        webhook: Option<String>,
        /// Print the payload instead of POSTing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Fire one sample alert to confirm webhook delivery.
    Test {
        /// Webhook URL override (default: `alert_webhook` from config).
        #[arg(long, value_name = "URL")]
        webhook: Option<String>,
        /// Print the payload instead of POSTing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum NewAction {
    /// Scaffold a new signed capability ("tool") and print the full author loop.
    Tool {
        /// Tool/capability name (e.g., "my-diagnostics").
        name: String,
        /// Language: rust, cpp, python, go.
        #[arg(long, default_value = "rust")]
        language: String,
        /// Output directory (default: current directory).
        #[arg(long)]
        output_dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum RegistryAction {
    /// Search for capabilities in the registry.
    Search {
        /// Search query (matches name, description, tags).
        query: String,
    },
    /// Install a capability from the registry.
    Install {
        /// Capability name.
        name: String,
        /// Specific version (default: latest).
        #[arg(long)]
        version: Option<String>,
    },
    /// Publish a signed capability to the registry.
    Publish {
        /// Path to the signed .wasm component.
        wasm_path: String,
        /// Short description.
        #[arg(long)]
        description: Option<String>,
        /// Tags (comma-separated).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Version to publish (overrides the adjacent signed manifest).
        #[arg(long, value_name = "SEMVER")]
        version: Option<String>,
        /// Language (overrides the manifest): rust, cpp, python, go.
        #[arg(long)]
        language: Option<String>,
    },
    /// List all capabilities in the local registry.
    List,
    /// Show details for a specific capability.
    Info {
        /// Capability name.
        name: String,
    },
}

#[derive(Subcommand)]
enum PeerAction {
    /// Register a known peer (robot, relay, or operator).
    Add {
        /// Human-readable name for this peer.
        name: String,
        /// The peer's full peer ID (12D3-...).
        peer_id: String,
        /// Relay multiaddr for reaching this peer.
        #[arg(long, short)]
        relay: Option<String>,
        /// Role: robot-agent, operator, or relay.
        #[arg(long, default_value = "robot-agent")]
        role: String,
    },
    /// Remove a registered peer.
    Remove {
        /// Peer name to remove.
        name: String,
    },
    /// List all registered peers.
    List,
    /// Show details for a specific peer.
    Show {
        /// Peer name.
        name: String,
    },
    /// Rename a registered peer.
    Rename {
        /// Current name.
        old_name: String,
        /// New name.
        new_name: String,
    },
    /// Reset trust for a peer (clear stored host key).
    TrustReset {
        /// Peer name.
        name: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration.
    Show,
    /// Set a configuration value.
    Set {
        /// Key to set (e.g., "default_relay", "host_key_policy").
        key: String,
        /// Value to set.
        value: String,
    },
    /// Initialize a default config file.
    Init {
        /// Overwrite existing config.
        #[arg(long)]
        force: bool,
    },
    /// Show the config file path.
    Path,
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Show the active robot policy.
    Show,
    /// Permit a capability pattern — the minimal edit, validated and written
    /// atomically (never leaves a broken policy.toml behind).
    Allow {
        /// Capability group (e.g. "ganglion:ros/interface").
        group: String,
        /// Pattern to allow (e.g. "/cmd_vel", "journald/**"). Refuses "**"
        /// unless --wide-open is passed.
        pattern: String,
        /// Access level for groups that distinguish one ("read_only" or
        /// "read_write").
        #[arg(long)]
        access: Option<String>,
        /// Explicitly permit an everything-matching pattern. Wide-open rules
        /// are what `gang policy lint` exists to flag; requiring this flag
        /// keeps them a deliberate act instead of a reflex.
        #[arg(long)]
        wide_open: bool,
        /// Time-box the widening (sudo-timestamp analog): a duration
        /// ("45m", "2h", "7d") or an RFC3339 instant. The engine ignores the
        /// pattern after expiry; `gang policy lint` flags the leftover. (#34)
        #[arg(long, value_name = "DURATION|RFC3339")]
        until: Option<String>,
        /// Free-text reason recorded in the policy change history (#36).
        #[arg(long)]
        reason: Option<String>,
    },
    /// Authorize a peer to deploy capabilities.
    AllowPeer {
        /// The operator's gang id (from `gang identity show`).
        peer_id: String,
        /// Free-text reason recorded in the policy change history (#36).
        #[arg(long)]
        reason: Option<String>,
    },
    /// Pre-flight a signed component against the local policy WITHOUT
    /// deploying: per-capability verdicts, each denial with its remedy. (#35)
    Check {
        /// Path to the signed component (.wasm with a .manifest.cbor sidecar).
        wasm_path: std::path::PathBuf,
        /// Explicit manifest path (default: `<wasm>.manifest.cbor`).
        #[arg(long)]
        manifest: Option<std::path::PathBuf>,
        /// Evaluate as this deploying peer (default: the local identity).
        #[arg(long, value_name = "GANG_ID")]
        as_peer: Option<String>,
    },
    /// Show the policy change history: who widened what, when, and why. (#36)
    History {
        /// Show at most this many entries (newest first).
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
    /// Review recent policy denials with the minimal rule that would permit
    /// each — firewall-log style.
    Denials {
        /// Show at most this many distinct denials.
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
    /// Flag over-broad rules that undermine default-deny (run it in CI or
    /// cron; exits non-zero with --strict when findings exist).
    Lint {
        /// Exit 1 when any finding exists.
        #[arg(long)]
        strict: bool,
    },
}

/// `gang telemetry` subcommands (ADR-026, TELEMETRY.md).
#[derive(Subcommand)]
pub enum TelemetryAction {
    /// Show whether telemetry is enabled, which opt-out layer applies if
    /// not, the anonymous id, and the endpoint.
    Status,
    /// Print, byte-for-byte, the payload the next daily checkpoint would
    /// send. Nothing is sent.
    Show,
    /// Enable telemetry in config.toml (environment opt-outs still win).
    On,
    /// Disable telemetry in config.toml. Recommended for production fleets.
    Off,
    /// Regenerate the anonymous id and clear pending counters.
    Reset,
}

#[derive(Subcommand)]
enum IdentityAction {
    /// Show your peer ID and public key.
    Show,
    /// Generate a new identity keypair.
    Generate {
        /// Force overwrite existing key.
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // A global `--data-dir` redirects the entire CLI's `~/.gang` lookups
    // (identity, peer registry, config, trust store) at the given directory via
    // GANG_HOME, so `gang --data-dir <dir> deploy …` drives a `gang up` fleet.
    // `gang up` resolves and sets this itself when the flag is omitted.
    if let Some(dir) = &cli.data_dir {
        // SAFETY: set once at startup, before any threads read the environment.
        unsafe {
            std::env::set_var("GANG_HOME", dir);
        }
    }

    // Initialize tracing. RUST_LOG wins when set; otherwise derive from
    // the -q / -v flags. Each verbosity level is genuinely distinct.
    let derived = if cli.quiet {
        "gang=error".to_string()
    } else {
        match cli.verbose {
            0 => "gang=info".to_string(),
            1 => "gang=debug".to_string(),
            2 => "gang=trace,gang_core=trace,gang_ros=trace,gang_libp2p=trace".to_string(),
            _ => "trace".to_string(),
        }
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(derived));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Telemetry (ADR-026): category + notify computed before dispatch; the
    // outcome recorded after. Operator-side only — the module's allowlist
    // excludes agent/join/relay/doctor/diagnose/test-archetype entirely, so
    // a robot or field-triage invocation never touches telemetry code.
    let telemetry_category = telemetry::command_category(&cli.command);
    let telemetry_notify = matches!(cli.format, OutputFormat::Text) && !cli.quiet;

    let result = dispatch(cli).await;
    telemetry::record_command(telemetry_category, result.is_ok(), telemetry_notify);
    result
}

/// The CLI command dispatcher (extracted from `main` so telemetry can record
/// each allowlisted command's outcome in exactly one place).
async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init { force, yes, json } => {
            commands::init(cli.data_dir.as_deref(), force, yes, json, &cli.format).await?
        }
        Commands::Pair {
            relay,
            name,
            expires,
            qr,
            timeout,
            json,
        } => {
            commands::pair(
                relay.as_deref(),
                name.as_deref(),
                expires.as_deref(),
                qr,
                timeout,
                json,
                &cli.format,
            )
            .await?
        }
        Commands::Join {
            token,
            name,
            once,
            timeout,
            json,
        } => commands::join(&token, name.as_deref(), once, timeout, json, &cli.format).await?,
        Commands::Identity { action } => {
            reject_json(&cli.format, "identity")?;
            match action {
                IdentityAction::Show => commands::identity_show().await?,
                IdentityAction::Generate { force } => commands::identity_generate(force).await?,
            }
        }
        Commands::Sign {
            wasm_path,
            key,
            name,
            version,
            capabilities,
            http_endpoints,
            credential_slots,
            exports,
        } => {
            reject_json(&cli.format, "sign")?;
            commands::sign(
                &wasm_path,
                key.as_deref(),
                name.as_deref(),
                &version,
                capabilities.as_deref(),
                &http_endpoints,
                &credential_slots,
                &exports,
            )
            .await?
        }
        Commands::Agent {
            config,
            data_dir,
            relay,
        } => commands::agent(config.as_deref(), &data_dir, relay.as_deref()).await?,
        Commands::Deploy {
            robot,
            wasm_path,
            manifest,
            peer,
            relay,
            timeout,
        } => {
            commands::deploy(
                &robot,
                &wasm_path,
                manifest.as_deref(),
                peer.as_deref(),
                relay.as_deref(),
                timeout,
                &cli.format,
            )
            .await?
        }
        Commands::Run {
            robot,
            cap_name,
            args,
            peer,
            relay,
            timeout,
            export,
        } => {
            commands::run(
                &robot,
                &cap_name,
                &args,
                peer.as_deref(),
                relay.as_deref(),
                timeout,
                export.as_deref(),
                &cli.format,
            )
            .await?
        }
        Commands::Caps {
            robot,
            peer,
            relay,
            timeout,
        } => {
            commands::caps(
                &robot,
                peer.as_deref(),
                relay.as_deref(),
                timeout,
                &cli.format,
            )
            .await?
        }
        Commands::Demo => commands::demo(&cli.format).await?,
        Commands::Up { port, force, json } => {
            commands::up(cli.data_dir.as_deref(), port, force, json, &cli.format).await?
        }
        Commands::Logs {
            robot,
            follow,
            since,
            peer,
            relay,
            events_transport,
        } => {
            commands::logs(
                &robot,
                follow,
                since.as_deref(),
                peer.as_deref(),
                relay.as_deref(),
                events_transport.map(Into::into),
                &cli.format,
            )
            .await?
        }
        Commands::TestArchetype { archetype } => commands::test_archetype(&archetype).await?,
        Commands::Diagnose { robot } => commands::diagnose(robot.as_deref(), &cli.format).await?,
        Commands::Doctor {
            relay,
            profile_out,
            profile_name,
            samples,
            uplink_kbit,
            downlink_kbit,
        } => {
            doctor::doctor(
                relay.as_deref(),
                &cli.format,
                profile_out.as_deref().map(|p| doctor::ProfileOut {
                    path: p.to_path_buf(),
                    name: profile_name.clone(),
                    samples: samples as usize,
                    uplink_kbit,
                    downlink_kbit,
                }),
            )
            .await?
        }
        Commands::Profiles => commands::profiles(&cli.format).await?,
        Commands::Mcp => mcp::serve(&cli.format).await?,
        Commands::Alert { action } => match action {
            AlertAction::Check {
                metric,
                value,
                webhook,
                dry_run,
            } => commands::alert_check(&metric, value, webhook.as_deref(), dry_run).await?,
            AlertAction::Test { webhook, dry_run } => {
                commands::alert_test(webhook.as_deref(), dry_run).await?
            }
        },
        Commands::TransportStats {
            robot,
            peer,
            relay,
            timeout,
        } => {
            commands::transport_stats(
                &robot,
                peer.as_deref(),
                relay.as_deref(),
                timeout,
                &cli.format,
            )
            .await?
        }
        Commands::Fetch { cid, output } => {
            commands::fetch_artifact(&cid, output.as_deref(), &cli.format).await?
        }
        Commands::Push { path, content_type } => {
            commands::push_artifact(&path, content_type.as_deref(), &cli.format).await?
        }
        Commands::Artifacts => commands::list_artifacts(&cli.format).await?,
        Commands::Capability { action } => match action {
            CapabilityAction::Scaffold {
                name,
                language,
                output_dir,
            } => {
                reject_json(&cli.format, "capability scaffold")?;
                commands::capability_scaffold(&name, &language, output_dir.as_deref()).await?
            }
        },
        Commands::New { what } => match what {
            NewAction::Tool {
                name,
                language,
                output_dir,
            } => {
                reject_json(&cli.format, "new tool")?;
                commands::new_tool(&name, &language, output_dir.as_deref()).await?
            }
        },
        Commands::Registry { action } => match action {
            RegistryAction::Search { query } => {
                commands::registry_search(&query, &cli.format).await?
            }
            RegistryAction::Install { name, version } => {
                reject_json(&cli.format, "registry install")?;
                commands::registry_install(&name, version.as_deref(), &cli.format).await?
            }
            RegistryAction::Publish {
                wasm_path,
                description,
                tags,
                version,
                language,
            } => {
                reject_json(&cli.format, "registry publish")?;
                commands::registry_publish(
                    &wasm_path,
                    description.as_deref(),
                    tags.as_deref(),
                    version.as_deref(),
                    language.as_deref(),
                    &cli.format,
                )
                .await?
            }
            RegistryAction::List => commands::registry_list(&cli.format).await?,
            RegistryAction::Info { name } => commands::registry_info(&name, &cli.format).await?,
        },
        Commands::Peer { action } => match action {
            PeerAction::Add {
                name,
                peer_id,
                relay,
                role,
            } => commands::peer_add(&name, &peer_id, relay.as_deref(), &role, &cli.format).await?,
            PeerAction::Remove { name } => commands::peer_remove(&name, &cli.format).await?,
            PeerAction::List => commands::peer_list(&cli.format).await?,
            PeerAction::Show { name } => commands::peer_show(&name, &cli.format).await?,
            PeerAction::Rename { old_name, new_name } => {
                commands::peer_rename(&old_name, &new_name, &cli.format).await?
            }
            PeerAction::TrustReset { name } => {
                commands::peer_trust_reset(&name, &cli.format).await?
            }
        },
        Commands::Policy { action } => match action {
            PolicyAction::Show => commands::policy_show(&cli.format).await?,
            PolicyAction::Allow {
                group,
                pattern,
                access,
                wide_open,
                until,
                reason,
            } => {
                commands::policy_allow(
                    &group,
                    &pattern,
                    access.as_deref(),
                    wide_open,
                    until.as_deref(),
                    reason.as_deref(),
                    &cli.format,
                )
                .await?
            }
            PolicyAction::AllowPeer { peer_id, reason } => {
                commands::policy_allow_peer(&peer_id, reason.as_deref(), &cli.format).await?
            }
            PolicyAction::Check {
                wasm_path,
                manifest,
                as_peer,
            } => {
                commands::policy_check(
                    &wasm_path,
                    manifest.as_deref(),
                    as_peer.as_deref(),
                    &cli.format,
                )
                .await?
            }
            PolicyAction::History { last } => commands::policy_history(last, &cli.format).await?,
            PolicyAction::Denials { last } => commands::policy_denials(last, &cli.format).await?,
            PolicyAction::Lint { strict } => commands::policy_lint(strict, &cli.format).await?,
        },
        Commands::Config { action } => match action {
            ConfigAction::Show => commands::config_show(&cli.format).await?,
            ConfigAction::Set { key, value } => {
                commands::config_set(&key, &value, &cli.format).await?
            }
            ConfigAction::Init { force } => commands::config_init(force, &cli.format).await?,
            ConfigAction::Path => commands::config_path().await?,
        },
        Commands::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "gang", &mut std::io::stdout());
        }
        Commands::Status { html } => commands::status(&cli.format, html.as_deref()).await?,
        Commands::List => commands::list(&cli.format).await?,
        Commands::Tui {
            robot,
            frames,
            no_input,
            events_transport,
        } => {
            tui::tui(
                robot.as_deref(),
                frames,
                no_input,
                events_transport.map(Into::into),
                &cli.format,
            )
            .await?
        }
        Commands::Connect {
            robot,
            prefer_transport: _,
            events_transport,
        } => commands::connect(&robot, events_transport.map(Into::into), &cli.format).await?,
        Commands::View {
            robot,
            port,
            topics,
            profile,
            events_transport,
        } => {
            commands::view(
                &robot,
                port,
                &topics.unwrap_or_default(),
                profile.as_deref(),
                events_transport.map(Into::into),
            )
            .await?
        }
        Commands::Telemetry { action } => {
            reject_json(&cli.format, "telemetry")?;
            telemetry::telemetry_cli(&action)?
        }
        Commands::Relay {
            listen_addr,
            port,
            metrics_port,
            data_dir,
        } => commands::relay(listen_addr, port, metrics_port, data_dir.as_deref()).await?,
    }

    Ok(())
}
