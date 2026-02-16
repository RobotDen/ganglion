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
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Establish a session with a robot via relay.
    Connect {
        /// Robot name or peer ID.
        robot: String,
    },

    /// List reachable robots in the fleet.
    List,

    /// List capabilities installed on a robot.
    Caps {
        /// Robot name or peer ID.
        robot: String,
    },

    /// Deploy a signed capability to a robot.
    Deploy {
        /// Robot name or peer ID.
        robot: String,
        /// Path to the signed .wasm component.
        wasm_path: String,
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

    /// Stream robot logs.
    Logs {
        /// Robot name or peer ID.
        robot: String,
        /// Follow log output (like tail -f).
        #[arg(long)]
        follow: bool,
    },

    /// Sign a WASM component with your identity key.
    Sign {
        /// Path to the .wasm component to sign.
        wasm_path: String,
        /// Path to the signing key (default: ~/.gang/identity.key).
        #[arg(long)]
        key: Option<String>,
    },

    /// Manage peer identity.
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },

    /// Run a local test harness scenario.
    TestArchetype {
        /// Archetype to test: open-warehouse, nat-office, enterprise-dmz, mobile-cgnat.
        archetype: String,
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
        _ => "gang=trace,gang_core=trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    match cli.command {
        Commands::Identity { action } => match action {
            IdentityAction::Show => cmd_identity_show().await?,
            IdentityAction::Generate { force } => cmd_identity_generate(force).await?,
        },
        Commands::Connect { robot } => {
            tracing::info!("Connecting to {robot}...");
            eprintln!("gang connect: not yet implemented (Phase 2)");
        }
        Commands::List => {
            eprintln!("gang list: not yet implemented (Phase 2)");
        }
        Commands::Caps { robot } => {
            tracing::info!("Listing capabilities on {robot}...");
            eprintln!("gang caps: not yet implemented (Phase 5)");
        }
        Commands::Deploy { robot, wasm_path } => {
            tracing::info!("Deploying {wasm_path} to {robot}...");
            eprintln!("gang deploy: not yet implemented (Phase 5)");
        }
        Commands::Run {
            robot,
            cap_name,
            args,
        } => {
            tracing::info!("Running {cap_name} on {robot} with args {args:?}...");
            eprintln!("gang run: not yet implemented (Phase 5)");
        }
        Commands::Logs { robot, follow } => {
            tracing::info!("Streaming logs from {robot} (follow={follow})...");
            eprintln!("gang logs: not yet implemented (Phase 4)");
        }
        Commands::Sign { wasm_path, key } => {
            cmd_sign(&wasm_path, key.as_deref()).await?;
        }
        Commands::TestArchetype { archetype } => {
            tracing::info!("Testing archetype: {archetype}...");
            eprintln!("gang test-archetype: not yet implemented (Phase 7)");
        }
    }

    Ok(())
}

async fn cmd_identity_show() -> anyhow::Result<()> {
    let key_path = gang_core::identity::default_key_path();
    if !key_path.exists() {
        eprintln!(
            "No identity found. Run `gang identity generate` first.\n\
             Expected key at: {}",
            key_path.display()
        );
        std::process::exit(1);
    }

    let keypair = gang_core::identity::Keypair::load(&key_path)?;
    println!("Peer ID:    {}", keypair.peer_id());
    println!("Public key: {}", hex::encode(keypair.public_key().as_bytes()));
    println!("Key file:   {}", key_path.display());
    Ok(())
}

async fn cmd_identity_generate(force: bool) -> anyhow::Result<()> {
    let key_path = gang_core::identity::default_key_path();
    if key_path.exists() && !force {
        eprintln!(
            "Identity already exists at {}.\n\
             Use --force to overwrite.",
            key_path.display()
        );
        std::process::exit(1);
    }

    let keypair = gang_core::identity::Keypair::generate();
    keypair.save(&key_path)?;
    println!("Generated new identity:");
    println!("  Peer ID:  {}", keypair.peer_id());
    println!("  Key file: {}", key_path.display());
    Ok(())
}

async fn cmd_sign(wasm_path: &str, key_path: Option<&str>) -> anyhow::Result<()> {
    use gang_core::capability::CapabilityGroup;
    use gang_core::manifest::{ComponentManifest, ResourceLimits, SignedManifest};

    let key_path = key_path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(gang_core::identity::default_key_path);

    if !key_path.exists() {
        anyhow::bail!(
            "Key not found at {}. Run `gang identity generate` first.",
            key_path.display()
        );
    }

    let wasm_path = std::path::Path::new(wasm_path);
    if !wasm_path.exists() {
        anyhow::bail!("Component not found: {}", wasm_path.display());
    }

    let keypair = gang_core::identity::Keypair::load(&key_path)?;
    let component_bytes = std::fs::read(wasm_path)?;
    let component_hash = blake3::hash(&component_bytes).to_hex().to_string();

    // For now, create a manifest with diagnostics capabilities.
    // In a real workflow, the manifest would be specified separately or
    // inferred from the component's WIT imports.
    let name = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let manifest = ComponentManifest {
        name: name.into(),
        version: "0.1.0".into(),
        declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }],
        author_peer_id: keypair.peer_id(),
        component_hash,
        limits: ResourceLimits::default(),
    };

    let signed = SignedManifest::sign(&manifest, &keypair)?;
    let manifest_path = wasm_path.with_extension("manifest.cbor");
    let cbor = signed.to_cbor()?;
    std::fs::write(&manifest_path, &cbor)?;

    println!("Signed component: {}", wasm_path.display());
    println!("  Manifest: {}", manifest_path.display());
    println!("  Author:   {}", keypair.peer_id());
    println!("  Hash:     {}", manifest.component_hash);
    Ok(())
}
