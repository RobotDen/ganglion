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

    /// List reachable robots in the fleet.
    List,

    /// Establish a session with a robot via relay.
    Connect {
        /// Robot name or peer ID.
        robot: String,
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
        Commands::List => {
            eprintln!("gang list: requires a running relay connection (not yet implemented)");
            eprintln!("Use `gang demo` for a self-contained local demo.");
        }
        Commands::Connect { robot: _ } => {
            eprintln!("gang connect: requires a running relay (not yet implemented)");
            eprintln!("Use `gang demo` for a self-contained local demo.");
        }
    }

    Ok(())
}
