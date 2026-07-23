mod commands;

use clap::{CommandFactory, Parser, Subcommand};

/// gang — reach, observe, and act on ROS 2 robots behind hostile networks.
///
/// Ganglion provides the connectivity and tool-execution substrate for fleet
/// operators to reach robots deployed inside networks they don't own.
#[derive(Parser)]
#[command(name = "gang", version, about, long_about = None)]
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

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
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

#[derive(Subcommand)]
enum Commands {
    /// Manage peer identity.
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },

    /// Sign a WASM component with your identity key.
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
        /// "diagnostics,logs,ros,fs,artifacts,process,network,metrics").
        /// If omitted, a permissive default set is used with a warning.
        #[arg(long, value_delimiter = ',')]
        capabilities: Option<Vec<String>>,
    },

    /// Run the robot agent (for development/testing).
    /// Starts a local Ganglion agent that listens for operator connections.
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
    },

    /// Invoke an installed capability on a robot.
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
    },

    /// List capabilities installed on a robot.
    Caps {
        /// Robot name, abbreviated peer ID, or full peer ID.
        robot: String,
        /// Explicit peer ID (bypasses name/prefix resolution).
        #[arg(long, short = 'p')]
        peer: Option<String>,
        /// Relay multiaddr (overrides registry and config defaults).
        #[arg(long, short = 'r')]
        relay: Option<String>,
    },

    /// Stream robot logs. [WIP: requires relay]
    Logs {
        /// Robot name or peer ID.
        robot: String,
        /// Follow log output (like tail -f).
        #[arg(long)]
        follow: bool,
    },

    /// Run a local end-to-end demo: start agent, deploy diagnostics, invoke.
    Demo,

    /// Run a local test harness scenario (requires Docker).
    TestArchetype {
        /// Archetype: open-warehouse, nat-office, enterprise-dmz, mobile-cgnat.
        archetype: String,
    },

    /// Diagnose network environment — detect archetype and recommend transport config.
    Diagnose {
        /// Robot name or peer ID (optional — if omitted, probes local network).
        robot: Option<String>,
    },

    /// Show per-transport statistics for a connected peer. [WIP: simulated data until relay lands]
    TransportStats {
        /// Robot name or peer ID.
        robot: String,
    },

    /// Retrieve an artifact by CID from any reachable peer.
    Fetch {
        /// Content identifier of the artifact.
        cid: String,
        /// Output path (default: current directory).
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Publish a local file to the content store and announce its CID.
    Push {
        /// Path to the file to publish.
        path: String,
        /// Content type (e.g., "application/octet-stream").
        #[arg(long)]
        content_type: Option<String>,
    },

    /// List locally-stored artifacts.
    Artifacts,

    /// Scaffold, inspect, or manage capabilities.
    Capability {
        #[command(subcommand)]
        action: CapabilityAction,
    },

    /// Manage the capability registry.
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },

    /// Manage known peers (robots, relays, operators).
    Peer {
        #[command(subcommand)]
        action: PeerAction,
    },

    /// View or edit operator configuration (~/.gang/config.toml).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Show Ganglion status: version, identity, available and WIP capabilities.
    Status,

    /// List reachable robots in the fleet. [WIP: requires relay]
    List,

    /// Establish a session with a robot via relay. [WIP: requires relay]
    Connect {
        /// Robot name or peer ID.
        robot: String,
        /// Preferred transport order for happy-eyeballs selection.
        #[arg(long, value_delimiter = ',')]
        prefer_transport: Option<Vec<String>>,
    },

    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for.
        shell: clap_complete::Shell,
    },

    /// Run a circuit relay v2 server for NAT traversal.
    ///
    /// Starts a Ganglion node in relay-server mode, enabling robot agents
    /// behind NAT to accept inbound connections. This is the bootstrap relay
    /// described in the design spec (relay.gang.tafy.dev).
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

        /// Directory for the relay's persisted identity key (sets GANG_KEY_PATH).
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

    match cli.command {
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
        } => {
            reject_json(&cli.format, "sign")?;
            commands::sign(
                &wasm_path,
                key.as_deref(),
                name.as_deref(),
                &version,
                capabilities.as_deref(),
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
        } => {
            commands::deploy(
                &robot,
                &wasm_path,
                manifest.as_deref(),
                peer.as_deref(),
                relay.as_deref(),
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
        } => {
            commands::run(
                &robot,
                &cap_name,
                &args,
                peer.as_deref(),
                relay.as_deref(),
                &cli.format,
            )
            .await?
        }
        Commands::Caps { robot, peer, relay } => {
            commands::caps(&robot, peer.as_deref(), relay.as_deref(), &cli.format).await?
        }
        Commands::Demo => commands::demo(&cli.format).await?,
        Commands::Logs {
            robot: _,
            follow: _,
        } => commands::wip_stub("logs", &cli.format)?,
        Commands::TestArchetype { archetype } => commands::test_archetype(&archetype).await?,
        Commands::Diagnose { robot } => commands::diagnose(robot.as_deref(), &cli.format).await?,
        Commands::TransportStats { robot } => {
            commands::transport_stats(&robot, &cli.format).await?
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
        Commands::Status => commands::status(&cli.format).await?,
        Commands::List => commands::wip_stub("list", &cli.format)?,
        Commands::Connect {
            robot: _,
            prefer_transport: _,
        } => commands::wip_stub("connect", &cli.format)?,
        Commands::Relay {
            listen_addr,
            port,
            metrics_port,
            data_dir,
        } => commands::relay(listen_addr, port, metrics_port, data_dir.as_deref()).await?,
    }

    Ok(())
}
