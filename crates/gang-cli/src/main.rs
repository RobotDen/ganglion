mod commands;

use clap::{Parser, Subcommand};

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

    /// Verbosity level (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
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
        /// Component version.
        #[arg(long, default_value = "0.1.0")]
        version: String,
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
    },

    /// Deploy a signed capability to a robot.
    Deploy {
        /// Robot name or peer ID.
        robot: String,
        /// Path to the signed .wasm component.
        wasm_path: String,
        /// Path to the manifest (.manifest.cbor), auto-detected if adjacent.
        #[arg(long)]
        manifest: Option<String>,
    },

    /// Invoke an installed capability on a robot.
    Run {
        /// Robot name or peer ID.
        robot: String,
        /// Capability name to invoke.
        cap_name: String,
        /// Arguments to pass to the capability.
        args: Vec<String>,
    },

    /// List capabilities installed on a robot.
    Caps {
        /// Robot name or peer ID.
        robot: String,
    },

    /// Stream robot logs.
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

    /// Show per-transport statistics for a connected peer.
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

    /// Manage the capability registry.
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },

    /// List reachable robots in the fleet.
    List,

    /// Establish a session with a robot via relay.
    Connect {
        /// Robot name or peer ID.
        robot: String,
        /// Preferred transport order for happy-eyeballs selection.
        #[arg(long, value_delimiter = ',')]
        prefer_transport: Option<Vec<String>>,
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

    // Initialize tracing
    let filter = match cli.verbose {
        0 => "gang=info",
        1 => "gang=debug",
        _ => "gang=trace,gang_core=trace,gang_ros=trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    match cli.command {
        Commands::Identity { action } => match action {
            IdentityAction::Show => commands::identity_show().await?,
            IdentityAction::Generate { force } => commands::identity_generate(force).await?,
        },
        Commands::Sign {
            wasm_path,
            key,
            name,
            version,
        } => commands::sign(&wasm_path, key.as_deref(), name.as_deref(), &version).await?,
        Commands::Agent { config, data_dir } => {
            commands::agent(config.as_deref(), &data_dir).await?
        }
        Commands::Deploy {
            robot,
            wasm_path,
            manifest,
        } => commands::deploy(&robot, &wasm_path, manifest.as_deref(), &cli.format).await?,
        Commands::Run {
            robot,
            cap_name,
            args,
        } => commands::run(&robot, &cap_name, &args, &cli.format).await?,
        Commands::Caps { robot } => commands::caps(&robot, &cli.format).await?,
        Commands::Demo => commands::demo(&cli.format).await?,
        Commands::Logs { robot: _, follow: _ } => {
            eprintln!("gang logs: requires a running relay connection (not yet implemented)");
            eprintln!("Use `gang demo` for a self-contained local demo.");
        }
        Commands::TestArchetype { archetype } => {
            commands::test_archetype(&archetype).await?
        }
        Commands::Diagnose { robot } => {
            commands::diagnose(robot.as_deref(), &cli.format).await?
        }
        Commands::TransportStats { robot } => {
            commands::transport_stats(&robot, &cli.format).await?
        }
        Commands::Fetch { cid, output } => {
            commands::fetch_artifact(&cid, output.as_deref(), &cli.format).await?
        }
        Commands::Push { path, content_type } => {
            commands::push_artifact(&path, content_type.as_deref(), &cli.format).await?
        }
        Commands::Artifacts => {
            commands::list_artifacts(&cli.format).await?
        }
        Commands::Registry { action } => match action {
            RegistryAction::Search { query } => {
                commands::registry_search(&query, &cli.format).await?
            }
            RegistryAction::Install { name, version } => {
                commands::registry_install(&name, version.as_deref(), &cli.format).await?
            }
            RegistryAction::Publish {
                wasm_path,
                description,
                tags,
            } => {
                commands::registry_publish(
                    &wasm_path,
                    description.as_deref(),
                    tags.as_deref(),
                    &cli.format,
                )
                .await?
            }
            RegistryAction::List => {
                commands::registry_list(&cli.format).await?
            }
            RegistryAction::Info { name } => {
                commands::registry_info(&name, &cli.format).await?
            }
        },
        Commands::List => {
            eprintln!("gang list: requires a running relay connection (not yet implemented)");
            eprintln!("Use `gang demo` for a self-contained local demo.");
        }
        Commands::Connect { robot: _, prefer_transport: _ } => {
            eprintln!("gang connect: requires a running relay (not yet implemented)");
            eprintln!("Use `gang demo` for a self-contained local demo.");
        }
    }

    Ok(())
}
