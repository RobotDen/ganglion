use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::OutputFormat;

// --- Operator config ---

/// Operator configuration loaded from `~/.gang/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorConfig {
    /// Default relay multiaddr when --relay is not specified and the peer
    /// registry entry has no relay_addrs.
    pub default_relay: Option<String>,

    /// Identity verification policy: "strict" (default), "tofu", or "none".
    #[serde(default = "default_host_key_policy")]
    pub host_key_policy: String,
}

impl Default for OperatorConfig {
    /// Hand-written so a config-less environment (e.g. a fresh `gang up` fleet
    /// dir) gets the SAME defaults as a deserialized empty file. `#[derive]`
    /// would give `host_key_policy = ""`, which `verify_host_key` then rejects;
    /// the serde `default` attribute only fires during deserialization.
    fn default() -> Self {
        Self {
            default_relay: None,
            host_key_policy: default_host_key_policy(),
        }
    }
}

fn default_host_key_policy() -> String {
    "strict".to_string()
}

impl OperatorConfig {
    /// Load config from `~/.gang/config.toml`. Returns defaults if file is missing.
    pub fn load() -> Self {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        Self::load_from(&path)
    }

    /// Load config from a specific path. Returns defaults if the file is
    /// missing (silent). A present-but-malformed config is surfaced as a
    /// warning on stderr and then falls back to defaults, rather than being
    /// silently discarded.
    pub fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!(
                        "warning: ignoring malformed config at {} ({e}); using defaults. \
                         Fix the file or run `gang config init --force`.",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!(
                    "warning: could not read config at {} ({e}); using defaults.",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Save config to `~/.gang/config.toml`.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = gang_core::identity::default_config_dir().join("config.toml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)?;
        std::fs::write(&path, toml_str)?;
        Ok(())
    }
}

/// `gang status` — show version, identity, and capability summary.
pub async fn status(format: &OutputFormat) -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");

    // Check identity
    let key_path = gang_core::identity::default_key_path();
    let identity_status = if key_path.exists() {
        match gang_core::identity::Keypair::load(&key_path) {
            Ok(kp) => format!("{}", kp.peer_id()),
            Err(_) => "present but unreadable".to_string(),
        }
    } else {
        "not generated (run `gang identity generate`)".to_string()
    };

    // Registry count
    let reg_dir = registry_dir();
    let registry_count = match gang_core::registry::Registry::open(&reg_dir) {
        Ok(reg) => reg.list().len(),
        Err(_) => 0,
    };

    // Peer count
    let peer_registry =
        gang_core::identity::PeerRegistry::load(&gang_core::identity::default_registry_path())
            .unwrap_or_default();
    let peer_count = peer_registry.list().count();

    // Config
    let config = OperatorConfig::load();
    let config_path = gang_core::identity::default_config_dir().join("config.toml");

    // Data directories (outside ~/.gang)
    let artifact_dir = artifact_store_dir();

    let available = [
        "init",
        "pair",
        "join",
        "identity show",
        "identity generate",
        "sign",
        "agent",
        "deploy",
        "run",
        "caps",
        "demo",
        "up",
        "diagnose",
        "test-archetype",
        "push",
        "fetch",
        "artifacts",
        "capability scaffold",
        "registry search",
        "registry install",
        "registry publish",
        "registry list",
        "registry info",
        "peer add/remove/list/show/rename/trust-reset",
        "config show/set/init/path",
        "completions",
        "relay",
        "status",
        "logs",
        "connect",
        "list",
        "transport-stats",
    ];

    // Everything is built. `gang tui` (the dashboard atop the same event
    // subscription API) is the next milestone.
    let wip: [&str; 0] = [];

    match format {
        OutputFormat::Json => {
            let info = serde_json::json!({
                "version": version,
                "identity": identity_status,
                "key_path": key_path.display().to_string(),
                "registry_capabilities": registry_count,
                "registry_dir": reg_dir.display().to_string(),
                "artifact_store_dir": artifact_dir.display().to_string(),
                "registered_peers": peer_count,
                "config_path": config_path.display().to_string(),
                "default_relay": config.default_relay,
                "host_key_policy": config.host_key_policy,
                "available_commands": available,
                "wip_commands": wip,
            });
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        OutputFormat::Text => {
            println!("Ganglion v{version}");
            println!();
            println!("Identity:   {identity_status}");
            println!("Key file:   {}", key_path.display());
            println!("Registry:   {} capability(ies) registered", registry_count);
            println!("  dir:      {}", reg_dir.display());
            println!("Artifacts:  {}", artifact_dir.display());
            println!("Peers:      {} registered", peer_count);
            println!(
                "Config:     {}",
                if config_path.exists() {
                    config_path.display().to_string()
                } else {
                    "(not initialized — run `gang config init`)".to_string()
                }
            );
            if let Some(relay) = &config.default_relay {
                println!("Def. relay: {relay}");
            }
            println!();
            println!("Available commands:");
            for cmd in &available {
                println!("  gang {cmd}");
            }
        }
    }

    Ok(())
}

/// `gang identity show`
pub async fn identity_show() -> anyhow::Result<()> {
    let key_path = gang_core::identity::default_key_path();
    if !key_path.exists() {
        anyhow::bail!(
            "No identity found. Run `gang identity generate` first.\n\
             Expected key at: {}",
            key_path.display()
        );
    }

    let keypair = gang_core::identity::Keypair::load(&key_path)?;
    println!("Peer ID:    {}", keypair.peer_id());
    println!(
        "Public key: {}",
        hex::encode(keypair.public_key().as_bytes())
    );
    println!("Key file:   {}", key_path.display());
    Ok(())
}

/// `gang identity generate`
pub async fn identity_generate(force: bool) -> anyhow::Result<()> {
    let key_path = gang_core::identity::default_key_path();
    if key_path.exists() && !force {
        anyhow::bail!(
            "Identity already exists at {}. Use --force to overwrite.",
            key_path.display()
        );
    }

    let keypair = gang_core::identity::Keypair::generate();
    keypair.save(&key_path)?;
    println!("Generated new identity:");
    println!("  Peer ID:  {}", keypair.peer_id());
    println!("  Key file: {}", key_path.display());
    Ok(())
}

// --- First-run setup ---

/// Ask a yes/no question on the terminal, returning `default_yes` on a blank
/// line. Only called in interactive mode (a TTY is present).
fn prompt_yes_no(question: &str, default_yes: bool) -> anyhow::Result<bool> {
    use std::io::Write;
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(match input.trim().to_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Per-archetype "real deployment" hint for the next-steps panel. Every command
/// referenced here is a real `gang` subcommand.
fn archetype_deploy_hint(archetype: &gang_ros::archetype::NetworkArchetype) -> &'static str {
    use gang_ros::archetype::NetworkArchetype::*;
    match archetype {
        OpenWarehouse => {
            "Flat network: a direct QUIC path is available, but a relay still \
             gives robots behind egress controls a stable rendezvous."
        }
        NatOffice => {
            "Consumer NAT: run a relay; DCUtR will hole-punch to a direct QUIC \
             link after the first connection."
        }
        EnterpriseDmz => {
            "Enterprise DMZ: run the relay on TCP 443 (QUIC/UDP is likely \
             blocked) and plan for relay-only operation."
        }
        RegulatedFacility => {
            "Air-gapped: skip the relay. Sign capabilities here with `gang sign` \
             and move the signed bundle to the robot over approved media."
        }
        MobileCgnat => {
            "Mobile/CGNAT: run a relay and expect relay-only operation — \
             symmetric NAT defeats hole-punching."
        }
    }
}

/// `gang init` — guided first-run setup. Collapses the read-the-docs first-run
/// phase into one command: detect the network archetype, generate the operator
/// identity, write a default-deny policy + operator config, and print exactly
/// what to run next.
///
/// Interactive on a TTY (a couple of skippable prompts with safe defaults);
/// fully non-interactive when stdin is not a terminal or `--yes` is passed,
/// mirroring how `verify_host_key` degrades under `strict`. Existing files are
/// never overwritten without `--force`; the command reports what already
/// existed and continues (idempotent).
pub async fn init(
    _data_dir: Option<&str>,
    force: bool,
    yes: bool,
    json_flag: bool,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::identity::{Keypair, PeerId, default_config_dir, default_key_path};

    let json_output = json_flag || matches!(format, OutputFormat::Json);
    // Non-interactive when asked (`--yes`), when emitting JSON, or when stdin is
    // not a terminal (CI, pipes) — same rule the host-key prompt uses.
    let interactive = !yes && !json_output && std::io::IsTerminal::is_terminal(&std::io::stdin());

    // The global `--data-dir` has already pointed GANG_HOME here (main.rs), so
    // all default_* paths resolve inside the chosen home. `~/.gang` otherwise.
    let config_dir = default_config_dir();
    let key_path = default_key_path();
    let policy_path = config_dir.join("policy.toml");
    let config_path = config_dir.join("config.toml");
    std::fs::create_dir_all(&config_dir)
        .with_context(|| format!("creating data dir {}", config_dir.display()))?;

    if !json_output {
        println!("=== gang init — configuring Ganglion ===");
        println!();
        println!("Data dir: {}", config_dir.display());
        if !interactive && !yes {
            println!("(stdin is not a TTY — running non-interactively with defaults)");
        }
        println!();
    }

    // --- 1. Network archetype detection (reuses `gang diagnose`'s probes). ---
    let detection = gang_ros::archetype::detect_archetype();
    let transport_line = detection
        .recommendations
        .first()
        .cloned()
        .unwrap_or_else(|| "See `gang diagnose` for transport recommendations.".to_string());
    if !json_output {
        println!("[1/4] Network archetype");
        println!(
            "  Detected:  {} ({:.0}% confidence)",
            detection.archetype,
            detection.confidence * 100.0
        );
        println!("  Transport: {transport_line}");
        println!();
    }

    // --- 2. Operator identity (never clobber without --force). ---
    let mut identity_created = false;
    let identity_existed = key_path.exists();
    if !json_output {
        println!("[2/4] Operator identity");
    }
    if identity_existed && !force {
        let peer_id = match Keypair::load(&key_path) {
            Ok(kp) => kp.peer_id().to_string(),
            Err(_) => "present but unreadable".to_string(),
        };
        if !json_output {
            println!("  Already present: {peer_id}");
            println!("  Key file:        {}", key_path.display());
            println!("  (use --force to regenerate — this rotates your peer id)");
        }
    } else {
        let do_generate = if interactive {
            let q = if identity_existed {
                "Regenerate operator identity (rotates your peer id)?"
            } else {
                "Generate operator identity?"
            };
            prompt_yes_no(q, true)?
        } else {
            true
        };
        if do_generate {
            let keypair = Keypair::generate();
            keypair.save(&key_path)?;
            identity_created = true;
            if !json_output {
                println!(
                    "  {}: {}",
                    if identity_existed {
                        "Regenerated"
                    } else {
                        "Generated"
                    },
                    keypair.peer_id()
                );
                println!("  Key file:  {}", key_path.display());
            }
        } else if !json_output {
            println!("  Skipped — run `gang identity generate` when ready.");
        }
    }
    // The peer id to authorize in the policy (whatever identity exists now).
    let operator_id: Option<PeerId> = Keypair::load(&key_path).ok().map(|kp| kp.peer_id().clone());
    if !json_output {
        println!();
    }

    // --- 3. Policy + config (default-deny; never clobber without --force). ---
    if !json_output {
        println!("[3/4] Policy + config");
    }
    let write_policy_config = if interactive {
        prompt_yes_no("Write a default-deny policy and operator config?", true)?
    } else {
        true
    };
    let mut policy_created = false;
    let mut config_created = false;
    if write_policy_config {
        // Policy: genuinely default-deny (no active capability rules), with the
        // operator authorized to deploy and commented example rules to widen it.
        if policy_path.exists() && !force {
            if !json_output {
                println!("  Policy exists, kept:   {}", policy_path.display());
            }
        } else {
            // Without an identity we cannot name an authorized deployer; leave a
            // wildcard the user must narrow. This only happens if the user
            // declined identity generation above.
            let author = operator_id
                .clone()
                .unwrap_or_else(|| PeerId::parse("12D3-00000000000000000000000000000000").unwrap());
            std::fs::write(&policy_path, default_deny_policy(&author, false))
                .with_context(|| format!("writing policy {}", policy_path.display()))?;
            policy_created = true;
            if !json_output {
                println!("  Wrote default-deny policy: {}", policy_path.display());
            }
        }

        // Config: sane defaults, incl. host_key_policy = strict.
        if config_path.exists() && !force {
            if !json_output {
                println!("  Config exists, kept:   {}", config_path.display());
            }
        } else {
            OperatorConfig::default().save()?;
            config_created = true;
            if !json_output {
                println!(
                    "  Wrote operator config:     {}  (host_key_policy = strict)",
                    config_path.display()
                );
            }
        }
    } else if !json_output {
        println!("  Skipped — run `gang config init` and edit policy.toml when ready.");
    }
    if !json_output {
        println!();
    }

    // --- 4. Next steps, tailored to the detected archetype. ---
    use gang_ros::archetype::NetworkArchetype;
    let deploy_hint = archetype_deploy_hint(&detection.archetype);
    // Each step is (command, trailing-comment). Air-gapped facilities skip the
    // relay entirely; enterprise DMZ pins the relay to TCP 443.
    let deploy_steps: Vec<(String, &'static str)> =
        if matches!(detection.archetype, NetworkArchetype::RegulatedFacility) {
            vec![
                (
                    "gang sign <component.wasm> --capabilities <groups>".to_string(),
                    "# on this workstation",
                ),
                (
                    "# transfer <component>.wasm + .manifest.cbor over approved media".to_string(),
                    "",
                ),
                (
                    "gang deploy <name> <signed.wasm>".to_string(),
                    "# on the robot host",
                ),
            ]
        } else {
            let relay_port = if matches!(detection.archetype, NetworkArchetype::EnterpriseDmz) {
                "443"
            } else {
                "4001"
            };
            let relay_note = if matches!(detection.archetype, NetworkArchetype::EnterpriseDmz) {
                "# on a host both sides reach (TCP 443)"
            } else {
                "# on a host both sides reach"
            };
            vec![
                (format!("gang relay --port {relay_port}"), relay_note),
                (
                    "gang agent --relay <relay-multiaddr>".to_string(),
                    "# on the robot",
                ),
                (
                    "gang peer add <name> <robot-libp2p-id> --relay <relay-multiaddr>".to_string(),
                    "",
                ),
                (
                    "gang deploy <name> <signed.wasm>".to_string(),
                    "# from your workstation",
                ),
            ]
        };
    let mut next_commands: Vec<String> = vec!["gang up".to_string()];
    next_commands.extend(
        deploy_steps
            .iter()
            .map(|(cmd, _)| cmd.clone())
            .filter(|c| !c.starts_with('#')),
    );

    if json_output {
        let info = serde_json::json!({
            "status": "configured",
            "data_dir": config_dir.display().to_string(),
            "archetype": {
                "name": detection.archetype.to_string(),
                "confidence": detection.confidence,
                "transport": transport_line,
            },
            "identity": {
                "id": operator_id.as_ref().map(|p| p.as_str().to_string()),
                "key_path": key_path.display().to_string(),
                "created": identity_created,
                "existed": identity_existed,
            },
            "policy_path": policy_path.display().to_string(),
            "config_path": config_path.display().to_string(),
            "policy_created": policy_created,
            "config_created": config_created,
            "next_commands": next_commands,
        });
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    println!("[4/4] You're configured. What to run next");
    println!();
    println!("  # Try a live local fleet on loopback right now:");
    println!("  gang up");
    println!();
    println!("  # For a real deployment ({}):", detection.archetype);
    println!("  #   {deploy_hint}");
    for (cmd, comment) in &deploy_steps {
        if comment.is_empty() {
            println!("  {cmd}");
        } else {
            println!("  {cmd:<38} {comment}");
        }
    }
    println!();
    println!("  # Enrol a robot (gang pair is coming; today use peer add):");
    println!("  gang peer add <name> <robot-libp2p-id> --relay <relay-multiaddr>");
    println!();
    println!("Run `gang status` to review your configuration.");

    Ok(())
}

// --- Target resolution ---

/// Resolved target for a robot command.
pub struct ResolvedTarget {
    /// The full peer ID (if remote).
    pub peer_id: Option<gang_core::identity::PeerId>,
    /// The dialable libp2p peer id (base58), if known. Remote dispatch
    /// requires it — a `/p2p/` multiaddr component only accepts this form.
    pub libp2p_id: Option<String>,
    /// Relay multiaddr (if remote).
    pub relay_addr: Option<String>,
    /// Human-readable name (if registered).
    pub name: Option<String>,
    /// Whether this target is local-only (no network).
    pub is_local: bool,
}

/// Resolve a robot target string through the resolution chain:
/// 1. Explicit --peer flag (bypasses everything)
/// 2. Registered name in PeerRegistry
/// 3. Abbreviated peer ID prefix match (Docker-style)
/// 4. Full peer ID (37 chars)
/// 5. Local fallback (/tmp/gang-agent-{robot})
pub fn resolve_target(
    robot: &str,
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
) -> anyhow::Result<ResolvedTarget> {
    use gang_core::identity::{PeerId, PeerRegistry, default_registry_path};

    // Load config for default_relay fallback
    let config = OperatorConfig::load();

    // Relay resolution helper: CLI flag > peer registry entry > config default
    let resolve_relay =
        |explicit: Option<&str>, registry_addrs: Option<&Vec<String>>| -> Option<String> {
            explicit
                .map(String::from)
                .or_else(|| registry_addrs.and_then(|addrs| addrs.first().cloned()))
                .or_else(|| config.default_relay.clone())
        };

    // 1. Explicit --peer flag (accepts either the dialable libp2p id or a
    //    legacy gang id; only the former enables remote dispatch).
    if let Some(peer_str) = explicit_peer {
        let (peer_id, libp2p_id) =
            if let Some(ident) = gang_libp2p::identity_from_libp2p_str(peer_str) {
                (ident.gang_id, Some(ident.libp2p_id))
            } else {
                let id = PeerId::parse(peer_str).map_err(|e| {
                    anyhow::anyhow!(
                        "Invalid peer ID '{peer_str}': {e}. Expected the dialable libp2p id \
                         (12D3KooW…) or a gang id (12D3-<32 hex chars>)."
                    )
                })?;
                (id, None)
            };
        return Ok(ResolvedTarget {
            peer_id: Some(peer_id),
            libp2p_id,
            relay_addr: resolve_relay(explicit_relay, None),
            name: None,
            is_local: false,
        });
    }

    // Load registry for name and prefix lookups
    let registry_path = default_registry_path();
    let registry = PeerRegistry::load(&registry_path).unwrap_or_default();

    // 2. Registered name
    if let Some(entry) = registry.lookup(robot) {
        return Ok(ResolvedTarget {
            peer_id: Some(entry.peer_id.clone()),
            libp2p_id: entry.libp2p_id.clone(),
            relay_addr: resolve_relay(explicit_relay, Some(&entry.relay_addrs)),
            name: Some(robot.to_string()),
            is_local: false,
        });
    }

    // 3. Abbreviated peer ID prefix (must start with "12D3-")
    if robot.starts_with("12D3-") && robot.len() < 37 {
        let matches = registry.lookup_by_prefix(robot);
        match matches.len() {
            0 => anyhow::bail!(
                "No peer found matching prefix '{robot}'. Use `gang peer list` to see registered peers."
            ),
            1 => {
                let (name, entry) = matches[0];
                return Ok(ResolvedTarget {
                    peer_id: Some(entry.peer_id.clone()),
                    libp2p_id: entry.libp2p_id.clone(),
                    relay_addr: resolve_relay(explicit_relay, Some(&entry.relay_addrs)),
                    name: Some(name.to_string()),
                    is_local: false,
                });
            }
            n => {
                let mut msg = format!("Ambiguous peer ID prefix '{robot}' matches {n} peers:\n");
                for (name, entry) in &matches {
                    msg.push_str(&format!("  {} ({})\n", entry.peer_id, name));
                }
                msg.push_str("Provide a longer prefix to disambiguate.");
                anyhow::bail!(msg);
            }
        }
    }

    // 4. Full peer ID
    if robot.starts_with("12D3-") && robot.len() == 37 {
        let peer_id = PeerId::parse(robot).map_err(|e| {
            anyhow::anyhow!("Invalid peer ID '{robot}': {e}. Expected format: 12D3-<32 hex chars>")
        })?;
        return Ok(ResolvedTarget {
            peer_id: Some(peer_id),
            libp2p_id: None,
            relay_addr: resolve_relay(explicit_relay, None),
            name: None,
            is_local: false,
        });
    }

    // 4b. Full dialable libp2p id (base58 `12D3KooW…`) given directly.
    if let Some(ident) = gang_libp2p::identity_from_libp2p_str(robot) {
        return Ok(ResolvedTarget {
            peer_id: Some(ident.gang_id),
            libp2p_id: Some(ident.libp2p_id),
            relay_addr: resolve_relay(explicit_relay, None),
            name: None,
            is_local: false,
        });
    }

    // 5. Local fallback
    let local_path = PathBuf::from(format!("/tmp/gang-agent-{robot}"));
    if local_path.exists() {
        return Ok(ResolvedTarget {
            peer_id: None,
            libp2p_id: None,
            relay_addr: None,
            name: Some(robot.to_string()),
            is_local: true,
        });
    }

    // Nothing matched
    anyhow::bail!(
        "Unknown robot '{robot}'. Not a registered peer name, peer ID, or local agent.\n\
         Register with: gang peer add {robot} <peer-id> --relay <multiaddr>"
    );
}

// --- Peer registry commands ---

/// `gang peer add`
///
/// Accepts either id form:
/// - the dialable base58 libp2p id (`12D3KooW…`, printed by `gang agent` /
///   `gang relay`) — the gang trust id is derived from the Ed25519 key it
///   embeds, and BOTH ids are stored;
/// - a legacy gang id (`12D3-<32 hex>`) — stored without a dialable id, which
///   is enough for trust/policy references but NOT for remote dispatch.
pub async fn peer_add(
    name: &str,
    peer_id_str: &str,
    relay: Option<&str>,
    role_str: &str,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::identity::{PeerEntry, PeerId, PeerRegistry, Role, default_registry_path};

    let (peer_id, libp2p_id) =
        if let Some(ident) = gang_libp2p::identity_from_libp2p_str(peer_id_str) {
            // Dialable libp2p form: derive the canonical gang id from the
            // embedded Ed25519 key and keep both.
            (ident.gang_id, Some(ident.libp2p_id))
        } else if let Ok(id) = PeerId::parse(peer_id_str) {
            (id, None)
        } else {
            anyhow::bail!(
                "Invalid peer ID '{peer_id_str}'. Expected either the dialable libp2p id \
             (base58 `12D3KooW…`, printed by `gang agent`/`gang relay` at startup) \
             or a gang id (`12D3-` + 32 hex chars)."
            );
        };

    let role = match role_str {
        "robot-agent" | "robot" => Role::RobotAgent,
        "operator" => Role::Operator,
        "relay" => Role::Relay,
        _ => anyhow::bail!(
            "Unknown role: '{}'. Use: robot-agent, operator, or relay",
            role_str
        ),
    };

    let registry_path = default_registry_path();
    let mut registry = PeerRegistry::load(&registry_path)?;

    let entry = PeerEntry {
        peer_id: peer_id.clone(),
        role,
        relay_addrs: relay.into_iter().map(String::from).collect(),
        libp2p_id: libp2p_id.clone(),
    };

    registry.register(name.to_string(), entry);
    registry.save(&registry_path)?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "status": "registered",
                    "name": name,
                    "peer_id": peer_id.as_str(),
                    "libp2p_id": libp2p_id,
                    "role": role_str,
                })
            );
        }
        OutputFormat::Text => {
            println!("Registered peer '{name}':");
            println!("  Peer ID (gang identity): {peer_id}");
            match &libp2p_id {
                Some(id) => println!("  Peer ID (libp2p/dial):   {id}"),
                None => println!("  Peer ID (libp2p/dial):   (none)"),
            }
            println!("  Role:    {role_str}");
            if let Some(r) = relay {
                println!("  Relay:   {r}");
            }
            if libp2p_id.is_none() {
                println!();
                println!(
                    "note: registered with a legacy gang id only — remote dispatch \
                     (deploy/run/caps over a relay) needs the dialable libp2p id. \
                     Re-add with the `12D3KooW…` id printed by the agent/relay:\n\
                     \n  gang peer add {name} <libp2p-id> --relay <relay-multiaddr>"
                );
            }
        }
    }
    Ok(())
}

/// `gang peer remove`
pub async fn peer_remove(name: &str, format: &OutputFormat) -> anyhow::Result<()> {
    use gang_core::identity::{PeerRegistry, default_registry_path};

    let registry_path = default_registry_path();
    let mut registry = PeerRegistry::load(&registry_path)?;

    if registry.lookup(name).is_none() {
        anyhow::bail!("No peer registered with name '{name}'");
    }

    registry.remove(name);
    registry.save(&registry_path)?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::json!({"status": "removed", "name": name}));
        }
        OutputFormat::Text => {
            println!("Removed peer '{name}'");
        }
    }
    Ok(())
}

/// `gang peer list`
pub async fn peer_list(format: &OutputFormat) -> anyhow::Result<()> {
    use gang_core::identity::{PeerRegistry, default_registry_path};

    let registry_path = default_registry_path();
    let registry = PeerRegistry::load(&registry_path)?;

    let peers: Vec<_> = registry.list().collect();

    match format {
        OutputFormat::Json => {
            let entries: Vec<_> = peers
                .iter()
                .map(|(name, entry)| {
                    serde_json::json!({
                        "name": name,
                        "peer_id": entry.peer_id.as_str(),
                        "libp2p_id": entry.libp2p_id,
                        "role": format!("{}", entry.role),
                        "relay_addrs": entry.relay_addrs,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        OutputFormat::Text => {
            if peers.is_empty() {
                println!("No peers registered. Use `gang peer add` to register a peer.");
                return Ok(());
            }

            let header = format!(
                "{:<16} {:<16} {:<16} {:<14} {}",
                "NAME", "PEER ID", "DIAL ID", "ROLE", "RELAY"
            );
            println!("{header}");
            for (name, entry) in &peers {
                let abbrev = if entry.peer_id.as_str().len() > 16 {
                    &entry.peer_id.as_str()[..16]
                } else {
                    entry.peer_id.as_str()
                };
                let dial = entry
                    .libp2p_id
                    .as_deref()
                    .map(|id| if id.len() > 16 { &id[..16] } else { id })
                    .unwrap_or("(none)");
                let relay = entry
                    .relay_addrs
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("(none)");
                println!(
                    "{:<16} {:<16} {:<16} {:<14} {}",
                    name, abbrev, dial, entry.role, relay
                );
            }
        }
    }
    Ok(())
}

/// `gang peer show`
pub async fn peer_show(name: &str, format: &OutputFormat) -> anyhow::Result<()> {
    use gang_core::identity::{PeerRegistry, default_registry_path};

    let registry_path = default_registry_path();
    let registry = PeerRegistry::load(&registry_path)?;

    let entry = registry
        .lookup(name)
        .ok_or_else(|| anyhow::anyhow!("No peer registered with name '{name}'"))?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "name": name,
                    "peer_id": entry.peer_id.as_str(),
                    "libp2p_id": entry.libp2p_id,
                    "role": format!("{}", entry.role),
                    "relay_addrs": entry.relay_addrs,
                })
            );
        }
        OutputFormat::Text => {
            println!("Peer '{name}':");
            println!("  Peer ID:  {}", entry.peer_id);
            match &entry.libp2p_id {
                Some(id) => println!("  Dial ID:  {id}"),
                None => {
                    println!("  Dial ID:  (none — re-add with the libp2p id for remote dispatch)")
                }
            }
            println!("  Role:     {}", entry.role);
            if entry.relay_addrs.is_empty() {
                println!("  Relay:    (none)");
            } else {
                for addr in &entry.relay_addrs {
                    println!("  Relay:    {addr}");
                }
            }
        }
    }
    Ok(())
}

/// `gang peer rename`
pub async fn peer_rename(
    old_name: &str,
    new_name: &str,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::identity::{PeerRegistry, default_registry_path};

    let registry_path = default_registry_path();
    let mut registry = PeerRegistry::load(&registry_path)?;

    let entry = registry
        .remove(old_name)
        .ok_or_else(|| anyhow::anyhow!("No peer registered with name '{old_name}'"))?;

    registry.register(new_name.to_string(), entry);
    registry.save(&registry_path)?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({"status": "renamed", "old_name": old_name, "new_name": new_name})
            );
        }
        OutputFormat::Text => {
            println!("Renamed peer '{old_name}' → '{new_name}'");
        }
    }
    Ok(())
}

/// `gang peer trust-reset`
pub async fn peer_trust_reset(name: &str, format: &OutputFormat) -> anyhow::Result<()> {
    use gang_core::identity::{PeerRegistry, default_registry_path, default_trust_store_path};
    use gang_core::manifest::TrustStore;

    let registry_path = default_registry_path();
    let registry = PeerRegistry::load(&registry_path)?;

    let entry = registry
        .lookup(name)
        .ok_or_else(|| anyhow::anyhow!("No peer registered with name '{name}'"))?;

    let trust_path = default_trust_store_path();
    let mut trust_store = TrustStore::load(&trust_path)?;
    trust_store.remove(&entry.peer_id);
    trust_store.save(&trust_path)?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({"status": "trust_reset", "name": name, "peer_id": entry.peer_id.as_str()})
            );
        }
        OutputFormat::Text => {
            println!("Trust reset for peer '{name}' ({}).", entry.peer_id);
            println!("The next connection will prompt for identity verification.");
        }
    }
    Ok(())
}

// --- End peer registry commands ---

// --- Config commands ---

/// `gang config show`
pub async fn config_show(format: &OutputFormat) -> anyhow::Result<()> {
    let config = OperatorConfig::load();
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        OutputFormat::Text => {
            let path = gang_core::identity::default_config_dir().join("config.toml");
            println!("Config file: {}", path.display());
            println!();
            println!(
                "default_relay    = {}",
                config.default_relay.as_deref().unwrap_or("(not set)")
            );
            println!("host_key_policy  = {}", config.host_key_policy);
            println!();
            println!(
                "host_key_policy is enforced on remote connections (deploy/run/caps): \
                 strict = prompt on first connect, tofu = auto-accept first key, \
                 none = no verification (insecure). All policies except `none` \
                 hard-fail when a known robot's key changes."
            );
        }
    }
    Ok(())
}

/// `gang config set`
pub async fn config_set(key: &str, value: &str, format: &OutputFormat) -> anyhow::Result<()> {
    let mut config = OperatorConfig::load();
    match key {
        "default_relay" => {
            if value == "none" || value.is_empty() {
                config.default_relay = None;
            } else {
                config.default_relay = Some(value.to_string());
            }
        }
        "host_key_policy" => {
            if !["strict", "tofu", "none"].contains(&value) {
                anyhow::bail!(
                    "Invalid host_key_policy '{value}'. Valid options: strict, tofu, none"
                );
            }
            config.host_key_policy = value.to_string();
        }
        _ => {
            anyhow::bail!("Unknown config key '{key}'. Valid keys: default_relay, host_key_policy")
        }
    }
    config.save()?;
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({"status": "set", "key": key, "value": value})
            );
        }
        OutputFormat::Text => {
            println!("Set {key} = {value}");
        }
    }
    Ok(())
}

/// `gang config init`
pub async fn config_init(force: bool, format: &OutputFormat) -> anyhow::Result<()> {
    let path = gang_core::identity::default_config_dir().join("config.toml");
    if path.exists() && !force {
        anyhow::bail!(
            "Config file already exists at {}. Use --force to overwrite.",
            path.display()
        );
    }

    let default_config = OperatorConfig::default();
    default_config.save()?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({"status": "initialized", "path": path.display().to_string()})
            );
        }
        OutputFormat::Text => {
            println!("Initialized config at {}", path.display());
            println!("Edit the file or use `gang config set <key> <value>`.");
        }
    }
    Ok(())
}

/// `gang config path`
pub async fn config_path() -> anyhow::Result<()> {
    let path = gang_core::identity::default_config_dir().join("config.toml");
    println!("{}", path.display());
    Ok(())
}

// --- End config commands ---

// --- Identity verification (SSH-style TOFU) ---

/// Result of verifying a remote peer's identity.
pub enum HostKeyVerification {
    /// Peer is already trusted and key matches.
    Trusted,
    /// Peer was unknown but has been accepted (TOFU).
    Accepted,
    /// Identity verification is disabled.
    Skipped,
}

/// Compute an SSH-style fingerprint from a public key.
fn key_fingerprint(public_key: &[u8]) -> String {
    let hash = blake3::hash(public_key);
    format!("BLAKE3:{}", &hash.to_hex()[..32])
}

/// Verify the remote peer's public key using the configured host key policy.
///
/// - `strict`: TOFU on first connect (prompts interactively), hard fail on key change.
/// - `tofu`: auto-accept new keys without prompting, hard fail on key change.
/// - `none`: no verification (prints warning).
///
/// A robot's Ed25519 public key is embedded in its dialable libp2p id, so the
/// key being verified here is exactly the identity the Noise handshake will
/// enforce on the wire: libp2p refuses the connection if the peer at the
/// other end cannot prove possession of this key.
pub fn verify_host_key(
    peer_id: &gang_core::identity::PeerId,
    remote_public_key: &[u8],
    peer_name: Option<&str>,
) -> anyhow::Result<HostKeyVerification> {
    use gang_core::identity::default_trust_store_path;
    use gang_core::manifest::{TrustStore, TrustedPeer};

    let config = OperatorConfig::load();
    let trust_path = default_trust_store_path();
    let mut trust_store = TrustStore::load(&trust_path)?;

    match config.host_key_policy.as_str() {
        "none" => {
            eprintln!("WARNING: Host key verification is disabled (host_key_policy = \"none\").");
            eprintln!("This is insecure and should only be used for development/testing.");
            Ok(HostKeyVerification::Skipped)
        }
        policy @ ("strict" | "tofu") => {
            // A changed key implies a changed (key-derived) peer id, so the
            // per-peer-id lookup below can never see a mismatch. The real
            // "host key changed" signal is the NAME binding: this robot name
            // was previously trusted with a different identity.
            if let Some(name) = peer_name
                && let Some(existing) = trust_store.find_by_name(name)
                && (existing.peer_id != *peer_id || existing.public_key != remote_public_key)
            {
                let idx = trust_store.index_of(&existing.peer_id).unwrap_or(0);
                eprintln!("@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@");
                eprintln!("@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!    @");
                eprintln!("@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@");
                eprintln!("IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!");
                eprintln!("The Ed25519 host key for robot '{name}' has changed.");
                eprintln!("Previously trusted identity: {}", existing.peer_id);
                eprintln!("Identity now presented:      {peer_id}");
                eprintln!(
                    "Fingerprint for the new key: {}",
                    key_fingerprint(remote_public_key)
                );
                eprintln!(
                    "Add correct host key in {} to get rid of this message.",
                    trust_path.display()
                );
                eprintln!("Offending key stored at index {idx}.");
                eprintln!("Robot key verification failed.");
                anyhow::bail!(
                    "Host key verification failed for '{name}'. If this change is expected \
                     (e.g. the robot was re-imaged), run `gang peer trust-reset {name}` to \
                     clear the old key, then reconnect."
                );
            }

            if let Some(stored_key) = trust_store.get_public_key(peer_id) {
                // Known peer — verify key matches
                if stored_key == remote_public_key {
                    return Ok(HostKeyVerification::Trusted);
                }

                // Key mismatch!
                let display_name = peer_name
                    .map(|n| format!("'{n}' ({})", peer_id))
                    .unwrap_or_else(|| peer_id.to_string());
                let idx = trust_store.index_of(peer_id).unwrap_or(0);

                eprintln!("@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@");
                eprintln!("@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!    @");
                eprintln!("@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@");
                eprintln!("IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!");
                eprintln!("The Ed25519 host key for robot {display_name} has changed.");
                eprintln!(
                    "Fingerprint for the new key: {}",
                    key_fingerprint(remote_public_key)
                );
                eprintln!(
                    "Add correct host key in {} to get rid of this message.",
                    trust_path.display()
                );
                eprintln!("Offending key stored at index {idx}.");
                eprintln!("Robot key verification failed.");

                let reset_hint = if let Some(name) = peer_name {
                    format!("`gang peer trust-reset {name}`")
                } else {
                    format!(
                        "remove the entry for {} from {}",
                        peer_id,
                        trust_path.display()
                    )
                };
                anyhow::bail!(
                    "Host key verification failed for {display_name}. \
                     Run {reset_hint} to clear the old key, then reconnect."
                );
            }

            // Unknown peer — TOFU
            let fingerprint = key_fingerprint(remote_public_key);

            if policy == "strict" {
                eprintln!(
                    "The authenticity of robot '{}' can't be established.",
                    peer_id
                );
                eprintln!("Ed25519 key fingerprint is {fingerprint}.");

                // The strict policy needs a human at the terminal. Fail with
                // guidance instead of silently reading EOF from a pipe.
                if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    anyhow::bail!(
                        "host_key_policy is \"strict\" but stdin is not a terminal, so the \
                         first-connect confirmation cannot be asked. For non-interactive use, \
                         run `gang config set host_key_policy tofu` (auto-accept first key, \
                         hard-fail on change) or pre-provision the robot's key in the trust \
                         store."
                    );
                }

                // Read from stdin for interactive prompt
                eprint!("Are you sure you want to continue connecting (yes/no)? ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let answer = input.trim().to_lowercase();
                if answer != "yes" && answer != "y" {
                    anyhow::bail!("Host key verification aborted by user.");
                }
            } else {
                // tofu — auto-accept
                eprintln!(
                    "Auto-accepted host key for {} (fingerprint: {fingerprint}).",
                    peer_id
                );
            }

            // Store the key
            let name = peer_name.unwrap_or("unknown").to_string();
            trust_store.add(TrustedPeer {
                peer_id: peer_id.clone(),
                name,
                public_key: remote_public_key.to_vec(),
            });
            trust_store.save(&trust_path)?;

            eprintln!(
                "Warning: Permanently added '{}' ({fingerprint}) to the list of known robots.",
                peer_id
            );

            Ok(HostKeyVerification::Accepted)
        }
        other => {
            anyhow::bail!(
                "Unknown host_key_policy '{other}'. Valid options: strict, tofu, none. \
                 Set with: gang config set host_key_policy <policy>"
            );
        }
    }
}

// --- End identity verification ---

// --- Remote dispatch (ADR-020 Phase 32) ---

/// Default whole-dispatch timeout for `gang deploy`: component bytes can be
/// megabytes travelling over a relay circuit.
const DEPLOY_TIMEOUT_SECS: u64 = 60;
/// Default whole-dispatch timeout for `gang run` / `gang caps`.
pub(crate) const CONTROL_TIMEOUT_SECS: u64 = 30;

/// A fully-resolved remote robot target, ready to dial through its relay.
pub(crate) struct RemoteTarget {
    /// Canonical gang identity (trust store / policy / audit key).
    pub(crate) gang_id: gang_core::identity::PeerId,
    /// Dialable base58 libp2p id (embeds the same Ed25519 key).
    pub(crate) libp2p_id: String,
    /// The relay to route through (must end in `/p2p/<relay-libp2p-id>`).
    pub(crate) relay_addr: String,
    /// Registered name, when the target was resolved by name.
    pub(crate) name: Option<String>,
}

impl RemoteTarget {
    pub(crate) fn display(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| self.gang_id.to_string())
    }
}

/// Validate a resolved (non-local) target for remote dispatch and run the
/// SSH-style host-key verification gate before any connection is attempted.
pub(crate) fn prepare_remote(target: &ResolvedTarget) -> anyhow::Result<RemoteTarget> {
    let gang_id = target
        .peer_id
        .clone()
        .expect("remote target always carries a peer id");
    let display = target.name.clone().unwrap_or_else(|| gang_id.to_string());
    let readd_name = target.name.as_deref().unwrap_or("<name>");

    let Some(libp2p_id) = target.libp2p_id.clone() else {
        anyhow::bail!(
            "Remote dispatch to '{display}' needs the robot's dialable libp2p id \
             (base58 `12D3KooW…`), but only a legacy gang id is registered.\n\
             Re-register the robot with the ids the agent prints at startup:\n\
             \n    gang peer add {readd_name} <libp2p-id> --relay <relay-multiaddr>\n\
             \n(`gang agent` and `gang relay` print the libp2p id as \"Peer ID (libp2p/dial)\".)"
        );
    };

    let Some(relay_addr) = target.relay_addr.clone() else {
        anyhow::bail!(
            "No relay address known for '{display}'. Pass --relay <multiaddr>, store one \
             (`gang peer add {readd_name} {libp2p_id} --relay <multiaddr>`), or set a default \
             (`gang config set default_relay <multiaddr>`)."
        );
    };

    // Recover the Ed25519 key embedded in the dialable id; this is the exact
    // key libp2p's Noise handshake will hold the remote end to.
    let ident = gang_libp2p::identity_from_libp2p_str(&libp2p_id).ok_or_else(|| {
        anyhow::anyhow!(
            "Registered libp2p id '{libp2p_id}' for '{display}' is not a valid Ed25519 libp2p \
             peer id. Re-add the peer with the id printed by the agent/relay."
        )
    })?;
    if ident.gang_id != gang_id {
        anyhow::bail!(
            "Registry entry for '{display}' is inconsistent: its gang id is {gang_id} but the \
             identity embedded in its libp2p id derives to {}. Re-add the peer with \
             `gang peer add`.",
            ident.gang_id
        );
    }

    verify_host_key(&gang_id, &ident.ed25519_pubkey, target.name.as_deref())?;

    Ok(RemoteTarget {
        gang_id,
        libp2p_id,
        relay_addr,
        name: target.name.clone(),
    })
}

/// Build the circuit multiaddr for reaching `robot_libp2p_id` through
/// `relay_addr`.
///
/// The stored relay address already ends in `/p2p/<relay-libp2p-id>` (the
/// dialable form printed by `gang relay`); the circuit form appends
/// `/p2p-circuit/p2p/<robot-libp2p-id>`. Defensively strips a trailing
/// `/p2p/<robot-id>` or `/p2p-circuit` a user may have stored so the suffix
/// is never duplicated.
fn circuit_addr(relay_addr: &str, robot_libp2p_id: &str) -> String {
    let mut base = relay_addr.trim_end_matches('/').to_string();
    let robot_suffix = format!("/p2p/{robot_libp2p_id}");
    if let Some(stripped) = base.strip_suffix(&robot_suffix) {
        base = stripped.to_string();
    }
    if let Some(stripped) = base.strip_suffix("/p2p-circuit") {
        base = stripped.to_string();
    }
    format!("{base}/p2p-circuit/p2p/{robot_libp2p_id}")
}

/// Send one control message to a remote robot over the relay circuit and
/// return the decoded response. The entire exchange — transport construction,
/// relay dial, circuit dial, RPC — is bounded by `timeout`; a remote failure
/// is an error (non-zero exit), never a silent success.
async fn remote_dispatch(
    target: &RemoteTarget,
    message: gang_core::message::ControlMessage,
    timeout: std::time::Duration,
) -> anyhow::Result<gang_core::message::ControlMessage> {
    match tokio::time::timeout(timeout, remote_dispatch_inner(target, message, timeout)).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "timed out after {}s: robot '{}' not reachable via relay {} (is the agent \
             running, and did it connect to that relay?)",
            timeout.as_secs(),
            target.display(),
            target.relay_addr
        ),
    }
}

async fn remote_dispatch_inner(
    target: &RemoteTarget,
    message: gang_core::message::ControlMessage,
    rpc_timeout: std::time::Duration,
) -> anyhow::Result<gang_core::message::ControlMessage> {
    use gang_core::message::{ControlMessage, decode_message, encode_message};

    let display = target.display();
    let conn = establish_remote_connection(target, rpc_timeout).await?;

    let result = async {
        let request = encode_message(&message)
            .map_err(|e| anyhow::anyhow!("failed to encode control message: {e}"))?;
        let response_bytes = conn
            .transport
            .send_rpc_with_timeout(&target.gang_id, request, rpc_timeout)
            .await
            .map_err(|e| anyhow::anyhow!("control request to '{display}' failed: {e}"))?;

        if response_bytes.is_empty() {
            anyhow::bail!(
                "robot '{display}' sent no response on /ganglion/control/1.0 — is the agent \
                 actually serving (started with `gang agent -r <relay>`)?"
            );
        }
        let (response, _) = decode_message::<ControlMessage>(&response_bytes)
            .map_err(|e| anyhow::anyhow!("could not decode response from '{display}': {e}"))?;
        Ok(response)
    }
    .await;

    conn.close().await;
    result
}

/// A live operator transport connected to a robot through its relay circuit.
///
/// Holds the swarm worker's `JoinHandle` so callers can tear it down cleanly
/// via [`RemoteConnection::close`] once the exchange (dispatch, subscription
/// poll loop, …) is done.
pub(crate) struct RemoteConnection {
    pub(crate) transport: std::sync::Arc<gang_libp2p::Libp2pTransportAdapter>,
    event_loop: tokio::task::JoinHandle<Result<(), gang_libp2p::TransportError>>,
}

impl RemoteConnection {
    /// Shut the transport down and abort the swarm worker.
    pub(crate) async fn close(self) {
        let _ = gang_core::transport::TransportAdapter::shutdown(self.transport.as_ref()).await;
        self.event_loop.abort();
    }
}

/// Build an outbound operator transport and connect it to `target` through the
/// relay circuit, bounded by `connect_timeout`.
///
/// This is the single circuit-dial path shared by remote dispatch, the event
/// subscription commands (`logs`, `connect`), and `transport-stats`. It reuses
/// the same relay-dial → wait → circuit-dial → redial-until-connected sequence
/// (a failed circuit dial is never retried by the swarm, so we re-dial until
/// the robot's reservation is accepted).
pub(crate) async fn establish_remote_connection(
    target: &RemoteTarget,
    connect_timeout: std::time::Duration,
) -> anyhow::Result<RemoteConnection> {
    use std::sync::Arc;

    let display = target.display();

    // Operator identity: same key the rest of the CLI uses. The robot's trust
    // store and policy see the gang id derived from this key.
    let transport_config = gang_libp2p::Libp2pConfig {
        key_path: gang_core::identity::default_key_path(),
        // Outbound-only: the operator dials; it accepts no inbound connections
        // and requests no relay reservation.
        listen_addrs: vec![],
        ..Default::default()
    };
    let transport = Arc::new(gang_libp2p::Libp2pTransportAdapter::new(transport_config).await?);

    // The swarm worker must run before any dial can make progress.
    let loop_transport = Arc::clone(&transport);
    let event_loop = tokio::spawn(async move { loop_transport.run_event_loop().await });

    let connect = connect_via_circuit(&transport, target);
    match tokio::time::timeout(connect_timeout, connect).await {
        Ok(Ok(())) => Ok(RemoteConnection {
            transport,
            event_loop,
        }),
        Ok(Err(e)) => {
            let _ = gang_core::transport::TransportAdapter::shutdown(transport.as_ref()).await;
            event_loop.abort();
            Err(e)
        }
        Err(_) => {
            let _ = gang_core::transport::TransportAdapter::shutdown(transport.as_ref()).await;
            event_loop.abort();
            anyhow::bail!(
                "timed out connecting to '{display}' via relay {} (is the agent running, and \
                 did it connect to that relay?)",
                target.relay_addr
            )
        }
    }
}

/// Dial `target`'s relay and then the robot through the relay circuit, waiting
/// until the authenticated connection to the robot is up. Re-dials the circuit
/// periodically (a failed circuit dial is never retried by the swarm).
async fn connect_via_circuit(
    transport: &gang_libp2p::Libp2pTransportAdapter,
    target: &RemoteTarget,
) -> anyhow::Result<()> {
    let display = target.display();

    // Dial the relay first for a clear error when it is down, and wait for that
    // connection before requesting the circuit (a circuit dial racing the
    // in-flight relay dial can fail spuriously).
    transport
        .dial_multiaddr(&target.relay_addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot reach relay {}: {e}", target.relay_addr))?;
    loop {
        if !transport.connected_peers().await.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let circuit = circuit_addr(&target.relay_addr, &target.libp2p_id);
    transport.dial_multiaddr(&circuit).await.map_err(|e| {
        anyhow::anyhow!("failed to dial '{display}' via relay circuit {circuit}: {e}")
    })?;

    let redial_every = std::time::Duration::from_secs(2);
    let mut last_dial = std::time::Instant::now();
    loop {
        if transport
            .connected_peers()
            .await
            .iter()
            .any(|(id, _)| *id == target.gang_id)
        {
            break;
        }
        if last_dial.elapsed() >= redial_every {
            last_dial = std::time::Instant::now();
            // Non-fatal: the caller's timeout bounds the wait; re-dial each tick.
            let _ = transport.dial_multiaddr(&circuit).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(())
}

/// Print a capability's output bytes honestly: JSON is pretty-printed (with
/// the diagnostics renderer for recognizably diagnostics-shaped output in
/// text mode); anything else is printed as (lossy) text.
fn print_capability_output(output: &[u8], format: &OutputFormat) {
    match serde_json::from_slice::<serde_json::Value>(output) {
        Ok(val) => match format {
            OutputFormat::Json => match serde_json::to_string_pretty(&val) {
                Ok(pretty) => println!("{pretty}"),
                Err(_) => println!("{}", String::from_utf8_lossy(output)),
            },
            OutputFormat::Text => {
                let diagnostics_shaped = ["system_info", "network", "processes", "log_sources"]
                    .iter()
                    .any(|k| val.get(k).is_some());
                if diagnostics_shaped {
                    print_diagnostics(&val);
                } else {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&val)
                            .unwrap_or_else(|_| String::from_utf8_lossy(output).into_owned())
                    );
                }
            }
        },
        Err(_) => println!("{}", String::from_utf8_lossy(output)),
    }
}

// --- End remote dispatch ---

/// Parse a capability group short name (or fully-qualified name) into a
/// `CapabilityGroup` with permissive defaults for any pattern/allowlist fields.
fn parse_capability_group(spec: &str) -> anyhow::Result<gang_core::capability::CapabilityGroup> {
    use gang_core::capability::CapabilityGroup;
    let version = "1.0".to_string();
    let group = match spec.trim() {
        "diagnostics" | "ganglion:diagnostics/collect" => {
            CapabilityGroup::DiagnosticsCollect { version }
        }
        "logs" | "ganglion:logs/stream" => CapabilityGroup::LogStream {
            version,
            patterns: vec!["**".into()],
        },
        "ros" | "ganglion:ros/interface" => CapabilityGroup::RosInterface {
            version,
            patterns: vec![],
        },
        "fs" | "ganglion:fs/bounded" => CapabilityGroup::FsBounded {
            version,
            paths: vec![],
        },
        "artifacts" | "ganglion:artifacts/publish" => CapabilityGroup::ArtifactsPublish { version },
        "process" | "ganglion:process/spawn" => CapabilityGroup::ProcessSpawn {
            version,
            allowed_commands: vec![],
        },
        "network" | "ganglion:network/probe" => CapabilityGroup::NetworkProbe { version },
        "metrics" | "ganglion:metrics/emit" => CapabilityGroup::MetricsEmit { version },
        other => anyhow::bail!(
            "unknown capability group '{other}'. Valid: diagnostics, logs, ros, fs, \
             artifacts, process, network, metrics"
        ),
    };
    Ok(group)
}

/// `gang sign`
pub async fn sign(
    wasm_path: &str,
    key_path: Option<&str>,
    name: Option<&str>,
    version: &str,
    capabilities: Option<&[String]>,
) -> anyhow::Result<()> {
    use gang_core::capability::CapabilityGroup;
    use gang_core::manifest::{ComponentManifest, ResourceLimits, SignedManifest};

    let key_path = key_path
        .map(PathBuf::from)
        .unwrap_or_else(gang_core::identity::default_key_path);

    if !key_path.exists() {
        anyhow::bail!(
            "Key not found at {}. Run `gang identity generate` first.",
            key_path.display()
        );
    }

    let wasm_path = Path::new(wasm_path);
    if !wasm_path.exists() {
        anyhow::bail!("Component not found: {}", wasm_path.display());
    }

    let keypair = gang_core::identity::Keypair::load(&key_path)?;
    let component_bytes = std::fs::read(wasm_path)?;
    let component_hash = blake3::hash(&component_bytes).to_hex().to_string();

    let name = name.map(String::from).unwrap_or_else(|| {
        wasm_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    // Declared capabilities: from --capabilities when provided, otherwise a
    // permissive default set with a loud warning (real WIT-import extraction is
    // not yet wired — see CLI-06).
    let declared_capabilities = match capabilities {
        Some(specs) if !specs.is_empty() => specs
            .iter()
            .map(|s| parse_capability_group(s))
            .collect::<anyhow::Result<Vec<_>>>()?,
        _ => {
            eprintln!(
                "WARNING: no --capabilities provided. Falling back to a permissive default \
                 set (diagnostics + logs \"**\"). This is almost certainly NOT what the \
                 component actually needs. Pass --capabilities to declare them explicitly, \
                 e.g. --capabilities diagnostics,logs"
            );
            vec![
                CapabilityGroup::DiagnosticsCollect {
                    version: "1.0".into(),
                },
                CapabilityGroup::LogStream {
                    version: "1.0".into(),
                    patterns: vec!["**".into()],
                },
            ]
        }
    };

    let manifest = ComponentManifest {
        schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
        name: name.clone(),
        version: version.into(),
        declared_capabilities,
        author_peer_id: keypair.peer_id(),
        component_hash: component_hash.clone(),
        limits: ResourceLimits::default(),
        language: gang_core::registry::CapabilityLanguage::Rust,
        description: String::new(),
        tags: vec![],
        min_ganglion_version: None,
    };

    let signed = SignedManifest::sign(&manifest, &keypair)?;
    let manifest_path = wasm_path.with_extension("manifest.cbor");
    let cbor = signed.to_cbor()?;
    std::fs::write(&manifest_path, &cbor)?;

    println!("Signed component: {}", wasm_path.display());
    println!("  Name:     {name}");
    println!("  Version:  {version}");
    println!("  Manifest: {}", manifest_path.display());
    println!("  Author:   {}", keypair.peer_id());
    println!("  Hash:     {component_hash}");
    println!("  Capabilities:");
    for cap in &manifest.declared_capabilities {
        println!("    - {}", cap.qualified_name());
    }
    Ok(())
}

/// `gang agent` — run the robot agent.
pub async fn agent(
    config_path: Option<&str>,
    data_dir: &str,
    relay: Option<&str>,
) -> anyhow::Result<()> {
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;
    use std::sync::Arc;

    // Loading an AgentConfig from a file is not yet supported here (gang-ros
    // does not expose a deserializer). Be honest rather than silently ignoring.
    if let Some(path) = config_path {
        eprintln!(
            "warning: --config {path} is not yet supported; agent config file loading is \
             unavailable. Continuing with built-in dev defaults."
        );
    }

    let data_dir = PathBuf::from(data_dir);
    std::fs::create_dir_all(&data_dir)?;

    let config = AgentConfig {
        key_path: data_dir.join("identity.key"),
        policy_path: None, // permissive for dev
        trust_store_path: data_dir.join("trusted_peers.json"),
        capabilities_dir: data_dir.join("capabilities"),
        audit_log_path: data_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", data_dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    };

    let agent = Arc::new(RobotAgent::new(config)?);
    let peer_id = agent.peer_id().clone();

    println!("Robot agent started:");
    println!("  Peer ID:  {peer_id}");
    println!("  Data dir: {}", data_dir.display());
    println!("  Policy:   permissive (dev mode)");

    if let Some(relay_addr) = relay {
        println!("  Relay:    {relay_addr}");
        println!("  Mode:     remote (listening on /ganglion/control/1.0)");
        println!();

        // Create libp2p transport with agent identity. This also requests the
        // circuit reservation on the relay (once the event loop runs), which
        // is what makes this robot reachable *through* the relay.
        let transport_config = gang_libp2p::Libp2pConfig {
            key_path: data_dir.join("identity.key"),
            relay_addrs: vec![relay_addr.to_string()],
            ..Default::default()
        };

        let transport = Arc::new(gang_libp2p::Libp2pTransportAdapter::new(transport_config).await?);
        let libp2p_id = *transport.libp2p_peer_id();

        // The dialable (base58) id is what operators must register: only this
        // form can appear in a /p2p/ multiaddr component. The gang id above
        // identifies the robot in trust stores and policies.
        println!("Peer ID (libp2p/dial): {libp2p_id}");
        println!();
        println!("Register on operator machine:");
        println!("  gang peer add my-robot {libp2p_id} --relay {relay_addr}");
        println!();
        println!("Starting transport...");

        // Register the control protocol handler
        agent.serve(transport.as_ref()).await?;

        // Start the swarm event loop FIRST: dialing goes through the swarm
        // worker's command channel, so a dial issued before the loop runs
        // would queue (and previously deadlock) forever.
        let loop_transport = Arc::clone(&transport);
        let mut event_loop = tokio::spawn(async move { loop_transport.run_event_loop().await });

        // Dial the relay and confirm the connection in the background,
        // retrying with a warning on failure so an unreachable relay neither
        // hangs nor kills the agent — it keeps serving and keeps retrying.
        let dial_transport = Arc::clone(&transport);
        let relay_addr_owned = relay_addr.to_string();
        tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                attempt += 1;
                match dial_transport.dial_multiaddr(&relay_addr_owned).await {
                    Ok(()) => {
                        // The dial is queued by the swarm; confirm an actual
                        // connection was established before claiming success.
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(8);
                        let mut connected = false;
                        loop {
                            if !dial_transport.connected_peers().await.is_empty() {
                                connected = true;
                                break;
                            }
                            if std::time::Instant::now() >= deadline {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        }
                        if connected {
                            // Connected is not the same as REACHABLE: the robot
                            // is only dialable once the relay has accepted its
                            // circuit reservation (visible as a /p2p-circuit
                            // listen address). Wait for it before announcing
                            // readiness — operators (and the test harness) key
                            // off this line to start dispatching.
                            let deadline =
                                std::time::Instant::now() + std::time::Duration::from_secs(15);
                            loop {
                                let reserved = dial_transport
                                    .listen_addrs()
                                    .await
                                    .iter()
                                    .any(|a| a.contains("/p2p-circuit"));
                                if reserved {
                                    println!("Relay circuit reservation established.");
                                    println!(
                                        "Connected to relay. Waiting for operator connections..."
                                    );
                                    return;
                                }
                                if std::time::Instant::now() >= deadline {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            }
                            eprintln!(
                                "warning: connected to relay {relay_addr_owned} but no circuit \
                                 reservation yet (attempt {attempt}); retrying in 5s..."
                            );
                        } else {
                            eprintln!(
                                "warning: no connection to relay {relay_addr_owned} yet \
                                 (attempt {attempt}); retrying in 5s..."
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: failed to dial relay {relay_addr_owned}: {e} \
                             (attempt {attempt}); retrying in 5s..."
                        );
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });

        println!("Press Ctrl+C to stop.");

        // Run until the event loop ends or Ctrl+C arrives.
        tokio::select! {
            result = &mut event_loop => {
                match result {
                    Ok(Err(e)) => eprintln!("Transport event loop error: {e}"),
                    Err(e) => eprintln!("Transport event loop task failed: {e}"),
                    Ok(Ok(())) => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nAgent stopped.");
            }
        }
        event_loop.abort();
    } else {
        println!("  Mode:     local (no relay, use `gang deploy` for local testing)");
        println!();
        println!("Press Ctrl+C to stop.");

        tokio::signal::ctrl_c().await?;
        println!("\nAgent stopped.");
    }

    Ok(())
}

/// `gang deploy` — deploy a capability to a robot.
#[allow(clippy::too_many_arguments)] // CLI surface: each arg mirrors one flag
pub async fn deploy(
    robot: &str,
    wasm_path: &str,
    manifest_path: Option<&str>,
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    timeout_secs: Option<u64>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::message::{ControlMessage, InvokeStatus};
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;

    let wasm_path = Path::new(wasm_path);
    if !wasm_path.exists() {
        anyhow::bail!("Component not found: {}", wasm_path.display());
    }

    // Auto-detect manifest path
    let manifest_path = manifest_path
        .map(PathBuf::from)
        .unwrap_or_else(|| wasm_path.with_extension("manifest.cbor"));

    if !manifest_path.exists() {
        anyhow::bail!(
            "Manifest not found: {}\nSign the component first: gang sign {}",
            manifest_path.display(),
            wasm_path.display()
        );
    }

    let component_bytes = std::fs::read(wasm_path)?;
    let manifest_cbor = std::fs::read(&manifest_path)?;

    let target = resolve_target(robot, explicit_peer, explicit_relay)?;

    if !target.is_local {
        // Remote dispatch over the relay circuit (ADR-020 Phase 32).
        let remote = prepare_remote(&target)?;
        let display = remote.display();

        // Decode the signed manifest locally for the message envelope (and to
        // fail fast on a malformed bundle before shipping megabytes).
        let signed = gang_core::manifest::SignedManifest::from_cbor(&manifest_cbor)
            .with_context(|| format!("decoding manifest {}", manifest_path.display()))?;
        let manifest = signed
            .verify_and_decode()
            .with_context(|| format!("verifying manifest {}", manifest_path.display()))?;

        let message = ControlMessage::DeployCapability {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            manifest_cbor,
            component_bytes,
            nonce: gang_core::message::fresh_nonce(),
            timestamp_ms: gang_core::message::unix_millis_now(),
        };

        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(DEPLOY_TIMEOUT_SECS));
        let response = remote_dispatch(&remote, message, timeout).await?;

        return match response {
            ControlMessage::InvokeResult {
                status: InvokeStatus::Success,
                output,
                ..
            } => {
                let deployed = String::from_utf8_lossy(&output).into_owned();
                match format {
                    OutputFormat::Json => println!(
                        "{}",
                        serde_json::json!({
                            "status": "deployed",
                            "name": deployed,
                            "robot": display,
                            "remote": true,
                        })
                    ),
                    OutputFormat::Text => {
                        println!("Deployed '{deployed}' to robot '{display}' (via relay)");
                    }
                }
                Ok(())
            }
            ControlMessage::Error { code, message, .. } => {
                anyhow::bail!("deploy to '{display}' rejected by robot ({code}): {message}")
            }
            other => anyhow::bail!("unexpected response from robot '{display}': {other:?}"),
        };
    }

    // Local agent path
    let data_dir = PathBuf::from(format!("/tmp/gang-agent-{robot}"));
    std::fs::create_dir_all(&data_dir)?;

    let config = AgentConfig {
        key_path: data_dir.join("identity.key"),
        policy_path: None,
        trust_store_path: data_dir.join("trusted_peers.json"),
        capabilities_dir: data_dir.join("capabilities"),
        audit_log_path: data_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", data_dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    };

    let agent = RobotAgent::new(config)?;
    let operator_kp =
        gang_core::identity::Keypair::load_or_generate(&gang_core::identity::default_key_path())?;

    let name = agent
        .deploy_capability(&manifest_cbor, &component_bytes, &operator_kp.peer_id())
        .await?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "status": "deployed",
                    "name": name,
                    "robot": robot,
                })
            );
        }
        OutputFormat::Text => {
            println!("Deployed '{name}' to robot '{robot}'");
        }
    }

    Ok(())
}

/// `gang run` — invoke a capability on a robot.
#[allow(clippy::too_many_arguments)] // CLI surface: each arg mirrors one flag
pub async fn run(
    robot: &str,
    cap_name: &str,
    args: &[String],
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    timeout_secs: Option<u64>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::message::{ControlMessage, InvokeStatus};
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;

    let target = resolve_target(robot, explicit_peer, explicit_relay)?;

    if !target.is_local {
        // Remote dispatch over the relay circuit (ADR-020 Phase 32).
        let remote = prepare_remote(&target)?;
        let display = remote.display();

        let request_id = gang_core::message::fresh_nonce();
        let message = ControlMessage::InvokeCapability {
            name: cap_name.to_string(),
            args: args.to_vec(),
            request_id: request_id.clone(),
            nonce: gang_core::message::fresh_nonce(),
            timestamp_ms: gang_core::message::unix_millis_now(),
        };

        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(CONTROL_TIMEOUT_SECS));
        let response = remote_dispatch(&remote, message, timeout).await?;

        return match response {
            ControlMessage::InvokeResult { status, output, .. } => {
                if matches!(status, InvokeStatus::Success) {
                    print_capability_output(&output, format);
                    Ok(())
                } else {
                    anyhow::bail!(
                        "invocation of '{cap_name}' on '{display}' finished with status \
                         {status:?}: {}",
                        String::from_utf8_lossy(&output)
                    )
                }
            }
            ControlMessage::Error { code, message, .. } => {
                anyhow::bail!(
                    "invocation of '{cap_name}' on '{display}' failed ({code}): {message}"
                )
            }
            other => anyhow::bail!("unexpected response from robot '{display}': {other:?}"),
        };
    }

    let data_dir = PathBuf::from(format!("/tmp/gang-agent-{robot}"));
    if !data_dir.exists() {
        anyhow::bail!(
            "No agent data found for robot '{robot}' at {}\n\
             Deploy a capability first: gang deploy {robot} <wasm-path>",
            data_dir.display()
        );
    }

    let config = AgentConfig {
        key_path: data_dir.join("identity.key"),
        policy_path: None,
        trust_store_path: data_dir.join("trusted_peers.json"),
        capabilities_dir: data_dir.join("capabilities"),
        audit_log_path: data_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", data_dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    };

    let agent = RobotAgent::new(config)?;
    let operator_kp =
        gang_core::identity::Keypair::load_or_generate(&gang_core::identity::default_key_path())?;

    let output = agent
        .invoke_capability(cap_name, args, &operator_kp.peer_id())
        .await?;

    match format {
        OutputFormat::Json => {
            // Output is already JSON
            let val: serde_json::Value = serde_json::from_slice(&output)?;
            println!("{}", serde_json::to_string_pretty(&val)?);
        }
        OutputFormat::Text => {
            let val: serde_json::Value = serde_json::from_slice(&output)?;
            print_diagnostics(&val);
        }
    }

    Ok(())
}

/// `gang caps` — list installed capabilities.
pub async fn caps(
    robot: &str,
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    timeout_secs: Option<u64>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::message::ControlMessage;
    use gang_ros::agent::{AgentConfig, RobotAgent};

    let target = resolve_target(robot, explicit_peer, explicit_relay)?;

    if !target.is_local {
        // Remote dispatch over the relay circuit (ADR-020 Phase 32).
        let remote = prepare_remote(&target)?;
        let display = remote.display();

        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(CONTROL_TIMEOUT_SECS));
        let response = remote_dispatch(&remote, ControlMessage::ListCapabilities, timeout).await?;

        return match response {
            ControlMessage::CapabilityList { capabilities } => {
                match format {
                    OutputFormat::Json => {
                        let list: Vec<serde_json::Value> = capabilities
                            .iter()
                            .map(|c| {
                                serde_json::json!({
                                    "name": c.name,
                                    "version": c.version,
                                    "author": c.author.as_str(),
                                    "capabilities": c.declared_capabilities,
                                })
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&list)?);
                    }
                    OutputFormat::Text => {
                        if capabilities.is_empty() {
                            println!("No capabilities installed on '{display}'");
                        } else {
                            println!("Capabilities on '{display}':");
                            for cap in &capabilities {
                                println!("  {} v{} (by {})", cap.name, cap.version, cap.author);
                                for group in &cap.declared_capabilities {
                                    println!("    - {group}");
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            ControlMessage::Error { code, message, .. } => {
                anyhow::bail!("listing capabilities on '{display}' failed ({code}): {message}")
            }
            other => anyhow::bail!("unexpected response from robot '{display}': {other:?}"),
        };
    }

    let data_dir = PathBuf::from(format!("/tmp/gang-agent-{robot}"));
    if !data_dir.exists() {
        anyhow::bail!("No agent data found for robot '{robot}'");
    }

    let config = AgentConfig {
        key_path: data_dir.join("identity.key"),
        policy_path: None,
        trust_store_path: data_dir.join("trusted_peers.json"),
        capabilities_dir: data_dir.join("capabilities"),
        audit_log_path: data_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![],
        log_allowed_sources: vec![],
    };

    let agent = RobotAgent::new(config)?;
    let caps = agent.list_capabilities().await;

    match format {
        OutputFormat::Json => {
            let list: Vec<serde_json::Value> = caps
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "version": c.version,
                        "author": c.author_peer_id.as_str(),
                        "capabilities": c.declared_capabilities.iter()
                            .map(|g| g.qualified_name())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        OutputFormat::Text => {
            if caps.is_empty() {
                println!("No capabilities installed on '{robot}'");
            } else {
                println!("Capabilities on '{robot}':");
                for cap in &caps {
                    println!(
                        "  {} v{} (by {})",
                        cap.name, cap.version, cap.author_peer_id
                    );
                    for group in &cap.declared_capabilities {
                        println!("    - {}", group.qualified_name());
                    }
                }
            }
        }
    }

    Ok(())
}

/// `gang demo` — self-contained local demo.
pub async fn demo(format: &OutputFormat) -> anyhow::Result<()> {
    use gang_core::capability::CapabilityGroup;
    use gang_core::manifest::{ComponentManifest, ResourceLimits, SignedManifest};
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;

    println!("=== Ganglion v{} Demo ===", env!("CARGO_PKG_VERSION"));
    println!();

    // 1. Generate identity if needed
    let key_path = gang_core::identity::default_key_path();
    let keypair = gang_core::identity::Keypair::load_or_generate(&key_path)?;
    println!("Operator identity: {}", keypair.peer_id());

    // 2. Create a simulated robot agent
    let data_dir = PathBuf::from("/tmp/gang-demo");
    if data_dir.exists() {
        std::fs::remove_dir_all(&data_dir)?;
    }
    std::fs::create_dir_all(&data_dir)?;

    let agent_config = AgentConfig {
        key_path: data_dir.join("robot.key"),
        policy_path: None,
        trust_store_path: data_dir.join("trusted_peers.json"),
        capabilities_dir: data_dir.join("capabilities"),
        audit_log_path: data_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", data_dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    };

    let agent = RobotAgent::new(agent_config)?;
    println!("Robot agent:       {}", agent.peer_id());
    println!();

    // 3. Create and sign a diagnostics capability
    println!("--- Signing diagnostics capability ---");
    let component_bytes = b"gang-capability-diagnostics-v0.1.0-demo";
    let component_hash = blake3::hash(component_bytes).to_hex().to_string();

    let manifest = ComponentManifest {
        schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
        name: "diagnostics".into(),
        version: "0.1.0".into(),
        declared_capabilities: vec![
            CapabilityGroup::DiagnosticsCollect {
                version: "1.0".into(),
            },
            CapabilityGroup::LogStream {
                version: "1.0".into(),
                patterns: vec!["**".into()],
            },
        ],
        author_peer_id: keypair.peer_id(),
        component_hash,
        limits: ResourceLimits::default(),
        language: gang_core::registry::CapabilityLanguage::Rust,
        description: "System diagnostics".into(),
        tags: vec!["diagnostics".into()],
        min_ganglion_version: None,
    };

    let signed = SignedManifest::sign(&manifest, &keypair)?;
    let manifest_cbor = signed.to_cbor()?;
    println!("  Component signed by {}", keypair.peer_id());
    println!();

    // 4. Deploy
    println!("--- Deploying to robot ---");
    let name = agent
        .deploy_capability(&manifest_cbor, component_bytes, &keypair.peer_id())
        .await?;
    println!("  Deployed: {name}");
    println!();

    // 5. List capabilities
    println!("--- Installed capabilities ---");
    let caps = agent.list_capabilities().await;
    for cap in &caps {
        println!("  {} v{} ({})", cap.name, cap.version, cap.author_peer_id);
    }
    println!();

    // 6. Invoke
    println!("--- Invoking diagnostics ---");
    let output = agent
        .invoke_capability("diagnostics", &[], &keypair.peer_id())
        .await?;

    let val: serde_json::Value = serde_json::from_slice(&output)?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&val)?);
        }
        OutputFormat::Text => {
            print_diagnostics(&val);
        }
    }

    println!();
    println!("--- Audit log ---");
    let audit_log = gang_core::audit::AuditLog::new(data_dir.join("audit.log"), 50 * 1024 * 1024);
    let records = audit_log.read_all()?;
    for record in &records {
        println!(
            "  {} invoked '{}' v{} at {} -> {:?}",
            record.operator_peer_id,
            record.component_name,
            record.component_version,
            record.started_at.format("%H:%M:%S"),
            record.exit_status,
        );
    }

    println!();
    println!("=== Demo complete ===");
    println!("Data stored at: {}", data_dir.display());
    println!("Clean up when done: rm -rf {}", data_dir.display());

    Ok(())
}

/// Build a real, restrictive default-deny agent policy that still permits the
/// signed sample's declared group (`ganglion:diagnostics/collect`) and
/// authorizes exactly the local operator to deploy. Everything else is denied
/// because the policy engine is default-deny: a capability group with no rule,
/// or a peer with no rule, is rejected. Commented examples show how to widen it.
/// Render a default-deny robot-agent `policy.toml` authorizing `operator` to
/// deploy. When `allow_diagnostics` is true the diagnostics group is permitted
/// by an ACTIVE rule (so `gang up`'s signed sample deploys); otherwise every
/// capability group is denied and the diagnostics rule appears only as a
/// commented example a user uncomments. Shared by `gang up` and `gang init`.
fn default_deny_policy(operator: &gang_core::identity::PeerId, allow_diagnostics: bool) -> String {
    // The diagnostics rule: active for `gang up` (it deploys a diagnostics
    // sample), commented-out for `gang init` (a genuinely empty default-deny).
    let diagnostics_block = if allow_diagnostics {
        "# Permit ONLY the diagnostics group the sample capability declares.\n\
         [[capability_rules]]\n\
         group = \"ganglion:diagnostics/collect\"\n\
         allowed_patterns = [\"**\"]\n\n"
            .to_string()
    } else {
        "# Example rules — uncomment (and adjust) one to allow a group.\n\
         # [[capability_rules]]\n\
         # group = \"ganglion:diagnostics/collect\"\n\
         # allowed_patterns = [\"**\"]\n#\n"
            .to_string()
    };
    format!(
        r#"# Ganglion robot-agent policy — DEFAULT DENY.
#
# The policy engine denies anything not explicitly listed here:
#   * a capability group with no [[capability_rules]] entry is rejected;
#   * a deploying peer with no matching [[peer_rules]] entry is rejected.
# This is a real restrictive policy, not the permissive dev fallback. Each
# capability group stays denied until you add (or uncomment) a rule for it.

{diagnostics_block}# [[capability_rules]]
# group = "ganglion:logs/stream"
# allowed_patterns = ["journald/**", "ros2/**"]
#
# [[capability_rules]]
# group = "ganglion:ros/interface"
# allowed_patterns = ["/diagnostics", "/rosout"]
# max_access = "read_only"
#
# [[capability_rules]]
# group = "ganglion:fs/bounded"
# allowed_patterns = ["/var/log/**"]

# Authorize exactly this operator to deploy. Replace with the gang id of any
# operator you trust, or "*" to allow any trusted peer.
[[peer_rules]]
peer_id = "{operator}"
can_deploy = true
"#
    )
}

/// `gang up` — stand up a real local fleet (relay + agent + signed sample).
///
/// The on-ramp between `gang demo` (self-contained, tears itself down) and a
/// hand-wired relay/agent/deploy. Everything runs as in-process tasks in this
/// one runtime — the same wiring the e2e harness (`tests/remote_dispatch.rs`)
/// uses — so teardown is a single `abort()` per task with no child processes,
/// no PATH assumptions, and no orphaned relays if the command is killed.
pub async fn up(
    data_dir: Option<&str>,
    port: Option<u16>,
    force: bool,
    json_flag: bool,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::capability::CapabilityGroup;
    use gang_core::identity::{PeerEntry, PeerRegistry, Role};
    use gang_core::manifest::{
        ComponentManifest, ResourceLimits, SignedManifest, TrustStore, TrustedPeer,
    };
    use gang_libp2p::{Libp2pConfig, Libp2pTransportAdapter};
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;
    use std::sync::Arc;

    let json_output = json_flag || matches!(format, OutputFormat::Json);

    // 1. Resolve the working directory. With `--data-dir` main.rs has already
    //    pointed GANG_HOME at it; without it we default to `<config>/up` and
    //    set GANG_HOME here so the peer registry and operator identity this
    //    command writes land in the fleet dir (and `gang --data-dir <dir> …`
    //    from another terminal reads exactly those files).
    let data_dir: PathBuf = match data_dir {
        Some(d) => PathBuf::from(d),
        None => gang_core::identity::default_config_dir().join("up"),
    };
    // SAFETY: set once, before any transport or registry access below; no other
    // thread reads the environment at this point.
    unsafe {
        std::env::set_var("GANG_HOME", &data_dir);
    }

    // 2. Honor --force / refuse to clobber an existing non-empty dir.
    let non_empty = data_dir
        .read_dir()
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if non_empty {
        if force {
            std::fs::remove_dir_all(&data_dir)
                .with_context(|| format!("resetting fleet dir {}", data_dir.display()))?;
        } else {
            anyhow::bail!(
                "fleet dir {} already exists and is not empty. Pass --force to reset it \
                 (this removes its keys, registry, and agent state).",
                data_dir.display()
            );
        }
    }
    let relay_dir = data_dir.join("relay");
    let robot_dir = data_dir.join("robot");
    std::fs::create_dir_all(&relay_dir)?;
    std::fs::create_dir_all(&robot_dir)?;

    if !json_output {
        println!("=== gang up — standing up a local fleet ===");
        println!();
        println!("Data dir: {}", data_dir.display());
    }

    // 3. Operator identity (== default_key_path() now that GANG_HOME is set).
    let operator = gang_core::identity::Keypair::load_or_generate(&data_dir.join("identity.key"))?;
    let operator_id = operator.peer_id();

    // 4. Start the loopback relay and capture its dialable multiaddr.
    let relay_listen = match port {
        Some(p) => format!("/ip4/127.0.0.1/tcp/{p}"),
        None => "/ip4/127.0.0.1/tcp/0".to_string(),
    };
    let relay = Arc::new(
        Libp2pTransportAdapter::new(Libp2pConfig {
            key_path: relay_dir.join("identity.key"),
            listen_addrs: vec![relay_listen],
            relay_server: true,
            ..Default::default()
        })
        .await
        .context("building the relay transport")?,
    );
    let relay_loop = Arc::clone(&relay);
    let relay_task = tokio::spawn(async move { relay_loop.run_event_loop().await });

    let relay_tcp = wait_for(
        std::time::Duration::from_secs(15),
        "the relay to report its loopback listen address",
        || {
            let relay = Arc::clone(&relay);
            async move {
                relay
                    .listen_addrs()
                    .await
                    .into_iter()
                    .find(|a| a.contains("/tcp/"))
            }
        },
    )
    .await?;
    let relay_addr = format!("{relay_tcp}/p2p/{}", relay.libp2p_peer_id());

    // 5. Write the robot's trust store (trust the operator so remote deploy is
    //    authorized, SEC-03) and its real default-deny policy.
    let mut trust = TrustStore::default();
    trust.add(TrustedPeer {
        peer_id: operator_id.clone(),
        name: "up-operator".into(),
        public_key: operator.public_key().to_bytes().to_vec(),
    });
    let trust_path = robot_dir.join("trusted_peers.json");
    trust.save(&trust_path)?;

    let policy_path = robot_dir.join("policy.toml");
    std::fs::write(&policy_path, default_deny_policy(&operator_id, true))
        .with_context(|| format!("writing policy {}", policy_path.display()))?;

    // 6. Start the robot agent, pointed at the relay, serving the control
    //    protocol. It binds no direct listener — reachable through the relay
    //    circuit only, exactly like a robot behind NAT.
    let agent = Arc::new(RobotAgent::new(AgentConfig {
        key_path: robot_dir.join("identity.key"),
        policy_path: Some(policy_path.clone()),
        trust_store_path: trust_path,
        capabilities_dir: robot_dir.join("capabilities"),
        audit_log_path: robot_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", robot_dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    })?);
    let robot_id = agent.peer_id().clone();

    let robot_transport = Arc::new(
        Libp2pTransportAdapter::new(Libp2pConfig {
            key_path: robot_dir.join("identity.key"),
            listen_addrs: vec![],
            relay_addrs: vec![relay_addr.clone()],
            ..Default::default()
        })
        .await
        .context("building the robot transport")?,
    );
    let robot_libp2p_id = robot_transport.libp2p_peer_id().to_string();

    agent
        .serve(robot_transport.as_ref())
        .await
        .context("agent failed to serve the control protocol")?;
    let robot_loop = Arc::clone(&robot_transport);
    let robot_task = tokio::spawn(async move { robot_loop.run_event_loop().await });

    // Dial the relay and wait for the circuit reservation — the robot is only
    // reachable once the relay accepts it (mirrors `gang agent`'s readiness).
    robot_transport
        .dial_multiaddr(&relay_addr)
        .await
        .context("robot dialing the relay")?;
    wait_for(
        std::time::Duration::from_secs(20),
        "the robot's relay circuit reservation",
        || {
            let t = Arc::clone(&robot_transport);
            async move {
                t.listen_addrs()
                    .await
                    .into_iter()
                    .find(|a| a.contains("p2p-circuit"))
            }
        },
    )
    .await?;
    if !json_output {
        println!("Relay circuit reservation established.");
    }

    // 7. Sign the one sample capability with the operator identity. Placeholder
    //    component bytes are served through the diagnostics broker path (the
    //    same bytes the demo/e2e use), so no wasm toolchain is required.
    let sample_wasm = data_dir.join("diagnostics.wasm");
    let component_bytes = b"gang-capability-diagnostics-v0.1.0-up".to_vec();
    std::fs::write(&sample_wasm, &component_bytes)?;
    let manifest = ComponentManifest {
        schema_version: gang_core::manifest::MANIFEST_SCHEMA_VERSION.into(),
        name: "diagnostics".into(),
        version: "0.1.0".into(),
        declared_capabilities: vec![CapabilityGroup::DiagnosticsCollect {
            version: "1.0".into(),
        }],
        author_peer_id: operator_id.clone(),
        component_hash: blake3::hash(&component_bytes).to_hex().to_string(),
        limits: ResourceLimits::default(),
        language: gang_core::registry::CapabilityLanguage::Rust,
        description: "System diagnostics (gang up sample)".into(),
        tags: vec!["diagnostics".into()],
        min_ganglion_version: None,
    };
    let signed = SignedManifest::sign(&manifest, &operator)?;
    let sample_manifest = sample_wasm.with_extension("manifest.cbor");
    std::fs::write(&sample_manifest, signed.to_cbor()?)?;

    // 8. Register the robot in the operator's peer list by its dialable id —
    //    equivalent to `gang peer add up-robot <libp2p-id> --relay <addr>`.
    let registry_path = gang_core::identity::default_registry_path();
    let mut registry = PeerRegistry::load(&registry_path)?;
    registry.register(
        "up-robot".into(),
        PeerEntry {
            peer_id: robot_id.clone(),
            role: Role::RobotAgent,
            relay_addrs: vec![relay_addr.clone()],
            libp2p_id: Some(robot_libp2p_id.clone()),
        },
    );
    registry.save(&registry_path)?;

    // 8b. Pre-provision the robot's host key in the OPERATOR trust store (the
    //     known-hosts file `verify_host_key` consults) so the printed commands
    //     connect straight away under the default "strict" policy — no TOFU
    //     prompt. This is safe precisely because `up` generated that key: the
    //     operator genuinely knows it. (Distinct from the robot's own trust
    //     store above, which trusts the operator as a deployer.)
    let robot_kp = gang_core::identity::Keypair::load(&robot_dir.join("identity.key"))?;
    let host_trust_path = gang_core::identity::default_trust_store_path();
    let mut host_trust = TrustStore::load(&host_trust_path)?;
    host_trust.add(TrustedPeer {
        peer_id: robot_id.clone(),
        name: "up-robot".into(),
        public_key: robot_kp.public_key().to_bytes().to_vec(),
    });
    host_trust.save(&host_trust_path)?;

    // 9. Report the fleet facts and the exact next commands.
    let dd = data_dir.display();
    let sample = sample_wasm.display();
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "up",
                "data_dir": data_dir.display().to_string(),
                "relay_addr": relay_addr,
                "operator_id": operator_id.as_str(),
                "robot": {
                    "name": "up-robot",
                    "gang_id": robot_id.as_str(),
                    "libp2p_id": robot_libp2p_id,
                },
                "sample_wasm": sample_wasm.display().to_string(),
                "sample_manifest": sample_manifest.display().to_string(),
                "next_commands": [
                    format!("gang --data-dir {dd} deploy up-robot {sample}"),
                    format!("gang --data-dir {dd} run up-robot diagnostics"),
                    format!("gang --data-dir {dd} caps up-robot"),
                    format!("gang --data-dir {dd} peer list"),
                ],
            }))?
        );
    } else {
        println!();
        println!("  ┌─────────────────────────────────────────────────────────────");
        println!("  │ Your fleet is up.");
        println!("  ├─────────────────────────────────────────────────────────────");
        println!("  │ data dir : {dd}");
        println!("  │ relay    : {relay_addr}");
        println!("  │ robot    : up-robot  ({robot_id})");
        println!("  │ sample   : {sample}  (signed: diagnostics)");
        println!("  └─────────────────────────────────────────────────────────────");
        println!();
        println!("Drive it from another terminal:");
        println!();
        println!("  gang --data-dir {dd} deploy up-robot {sample}");
        println!("  gang --data-dir {dd} run up-robot diagnostics");
        println!("  gang --data-dir {dd} caps up-robot");
        println!("  gang --data-dir {dd} peer list");
        println!();
        println!(
            "The agent enforces a default-deny policy ({}):",
            policy_path.display()
        );
        println!("  only the sample's diagnostics group is permitted; any other");
        println!("  capability group is denied at deploy time.");
        println!();
        println!("Ctrl-C tears the whole fleet down.");
        println!();
    }

    // 10. Serve until Ctrl-C, then tear every task down.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            if !json_output {
                println!("\nTearing down fleet…");
            }
        }
        r = relay_task => {
            if let Ok(Err(e)) = r { eprintln!("relay event loop error: {e}"); }
        }
    }
    robot_task.abort();

    Ok(())
}

/// Poll `probe` until it yields `Some(_)` or `timeout` elapses. Used by
/// `gang up` to wait on loopback network readiness with an honest error.
async fn wait_for<T, F, Fut>(
    timeout: std::time::Duration,
    what: &str,
    mut probe: F,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(v) = probe().await {
            return Ok(v);
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("timed out after {}s waiting for {what}", timeout.as_secs());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

// --- Pairing / one-line enrollment (issue #5) ---

/// Default time `gang pair` waits for a robot to enroll before giving up.
const PAIR_WAIT_SECS: u64 = 300;
/// Default overall budget for the `gang join` enrollment exchange.
const JOIN_TIMEOUT_SECS: u64 = 60;

/// Parse a short human duration like `15m`, `1h`, `90s`, or a bare number of
/// seconds. Used for `gang pair --expires`.
fn parse_duration(s: &str) -> anyhow::Result<std::time::Duration> {
    let s = s.trim();
    let (num, unit_secs): (&str, u64) = if let Some(v) = s.strip_suffix("ms") {
        // Sub-second precision is pointless for a token TTL; treat as an error
        // rather than silently rounding to zero.
        let _ = v;
        anyhow::bail!("token lifetime must be at least one second (got '{s}')");
    } else if let Some(v) = s.strip_suffix('s') {
        (v, 1)
    } else if let Some(v) = s.strip_suffix('m') {
        (v, 60)
    } else if let Some(v) = s.strip_suffix('h') {
        (v, 3600)
    } else {
        (s, 1)
    };
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration '{s}' (use e.g. 15m, 1h, 90s)"))?;
    if n == 0 {
        anyhow::bail!("token lifetime must be greater than zero");
    }
    Ok(std::time::Duration::from_secs(n * unit_secs))
}

/// A short, human-friendly default robot name derived from a gang id, e.g.
/// `robot-a1b2c3d4` from `12D3-a1b2c3d4…`.
fn default_robot_name(gang_id: &gang_core::identity::PeerId) -> String {
    let tail = gang_id
        .as_str()
        .strip_prefix("12D3-")
        .unwrap_or(gang_id.as_str());
    format!("robot-{}", &tail[..tail.len().min(8)])
}

/// Print the robot line as a terminal QR when asked. No `qrcode` crate is in the
/// workspace dependency table, so QR rendering is deferred rather than pulled in
/// without approval; this prints an honest note and the copy-paste line instead.
fn maybe_render_qr(requested: bool, robot_line: &str) {
    if !requested {
        return;
    }
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        eprintln!(
            "note: --qr is not available in this build (a terminal-QR dependency is not yet \
             in the workspace; tracked as a follow-up). Use the copy-paste line below:"
        );
    }
    // Whether or not a tty, the line itself is always the reliable path.
    println!("{robot_line}");
}

/// `gang pair` — mint a single-use token, print the one robot line, and wait
/// for the robot to dial out and enroll (design A: full auto-registration over
/// the relay circuit). The robot's identity is recorded from the wire, never a
/// self-report.
#[allow(clippy::too_many_arguments)]
pub async fn pair(
    relay: Option<&str>,
    name: Option<&str>,
    expires: Option<&str>,
    qr: bool,
    timeout: Option<u64>,
    json_flag: bool,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::identity::default_registry_path;
    use gang_core::message::{ControlMessage, decode_message, encode_message};
    use gang_core::pairing::{DEFAULT_TTL, PairingToken, authorize_enrollment};
    use gang_core::transport::{StreamHandler, TransportAdapter};
    use std::sync::Arc;

    let json_output = json_flag || matches!(format, OutputFormat::Json);

    // 1. Resolve the relay: flag > config default. Refuse to guess.
    let config = OperatorConfig::load();
    let relay_addr = relay
        .map(String::from)
        .or_else(|| config.default_relay.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no relay to pair through. Pass --relay <multiaddr> (the dialable relay id \
                 printed by `gang relay`/`gang up`), or set one with \
                 `gang config set default_relay <multiaddr>`."
            )
        })?;

    let ttl = match expires {
        Some(s) => parse_duration(s)?,
        None => DEFAULT_TTL,
    };
    let wait = std::time::Duration::from_secs(timeout.unwrap_or(PAIR_WAIT_SECS));

    // 2. Operator identity + a transport that reserves a circuit on the relay,
    //    so the robot can dial the operator *through* the relay (outbound-only
    //    on both ends — the Tailscale move).
    let key_path = gang_core::identity::default_key_path();
    // Ensure the operator identity exists on disk before the transport loads it.
    let operator_id = gang_core::identity::Keypair::load_or_generate(&key_path)?.peer_id();

    let transport = Arc::new(
        gang_libp2p::Libp2pTransportAdapter::new(gang_libp2p::Libp2pConfig {
            key_path: key_path.clone(),
            listen_addrs: vec![],
            relay_addrs: vec![relay_addr.clone()],
            ..Default::default()
        })
        .await
        .context("building the operator pairing transport")?,
    );
    let operator_libp2p_id = transport.libp2p_peer_id().to_string();

    // 3. Mint the token and render the one robot line.
    let now_ms = gang_core::message::unix_millis_now();
    let token = PairingToken::mint(&relay_addr, &operator_libp2p_id, now_ms, ttl);
    let encoded = token.encode();
    let robot_line = format!("gang join {encoded}");

    // Shared state: the minted token, a one-shot flag so a token enrolls exactly
    // once, and a channel to hand the recorded identity back to this task.
    let token = Arc::new(token);
    let consumed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (done_tx, mut done_rx) =
        tokio::sync::mpsc::channel::<(String, gang_core::identity::PeerId, String)>(1);

    // 4. Register the enrollment handler on the control protocol. It reads the
    //    wire-authenticated gang id off the stream — never trusting a claim —
    //    verifies the token, records the robot, and acknowledges.
    let name_hint = name.map(String::from);
    let registry_path = default_registry_path();
    let trust_path = gang_core::identity::default_trust_store_path();
    let handler_token = Arc::clone(&token);
    let handler_consumed = Arc::clone(&consumed);
    let handler_operator_id = operator_id.clone();

    let handler: StreamHandler = Box::new(move |mut stream| {
        let token = Arc::clone(&handler_token);
        let consumed = Arc::clone(&handler_consumed);
        let done_tx = done_tx.clone();
        let registry_path = registry_path.clone();
        let trust_path = trust_path.clone();
        let name_hint = name_hint.clone();
        let operator_id = handler_operator_id.clone();
        Box::pin(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let wire_gang_id = stream.remote_peer.clone();
            let mut buf = Vec::new();
            if stream.inner.read_to_end(&mut buf).await.is_err() {
                return;
            }
            let msg: ControlMessage = match decode_message(&buf) {
                Ok((m, _)) => m,
                Err(_) => return,
            };
            let (token_secret, req_name, reported_libp2p_id) = match msg {
                ControlMessage::Enroll {
                    token_secret,
                    name,
                    libp2p_id,
                } => (token_secret, name, libp2p_id),
                _ => {
                    let err = ControlMessage::Error {
                        request_id: None,
                        code: "unexpected".into(),
                        message: "pairing session expects an enroll message".into(),
                    };
                    if let Ok(b) = encode_message(&err) {
                        let _ = stream.inner.write_all(&b).await;
                    }
                    return;
                }
            };

            // Derive the gang id from the robot's *claimed* dialable id, so we
            // can prove it embeds the same key libp2p authenticated on the wire.
            let derived = gang_libp2p::identity_from_libp2p_str(&reported_libp2p_id);
            let now_ms = gang_core::message::unix_millis_now();

            let reject = |code: &str, message: String| -> ControlMessage {
                ControlMessage::Error {
                    request_id: None,
                    code: code.into(),
                    message,
                }
            };

            let response = match derived {
                None => reject(
                    "bad_identity",
                    "reported libp2p id is not a valid Ed25519 peer id".into(),
                ),
                Some(ident) => {
                    match authorize_enrollment(
                        &token,
                        &token_secret,
                        &wire_gang_id,
                        &ident.gang_id,
                        now_ms,
                    ) {
                        Err(e) => reject("rejected", e.to_string()),
                        Ok(()) => {
                            // Single-use: claim the token; a racing/replayed
                            // enrollment loses here and is rejected.
                            if consumed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                reject("already_used", "pairing token already used".into())
                            } else {
                                let robot_name = if req_name.trim().is_empty() {
                                    name_hint
                                        .clone()
                                        .unwrap_or_else(|| default_robot_name(&wire_gang_id))
                                } else {
                                    req_name.clone()
                                };
                                // Record the robot BY ITS WIRE-AUTHENTICATED id.
                                let recorded = record_paired_robot(
                                    &registry_path,
                                    &trust_path,
                                    &robot_name,
                                    &wire_gang_id,
                                    &ident.libp2p_id,
                                    &ident.ed25519_pubkey,
                                    &token.relay_addr,
                                );
                                match recorded {
                                    Err(e) => {
                                        // Undo the consume so a retry can succeed.
                                        consumed.store(false, std::sync::atomic::Ordering::SeqCst);
                                        reject("record_failed", format!("{e}"))
                                    }
                                    Ok(()) => {
                                        let _ = done_tx
                                            .send((
                                                robot_name.clone(),
                                                wire_gang_id.clone(),
                                                ident.libp2p_id.clone(),
                                            ))
                                            .await;
                                        ControlMessage::Enrolled {
                                            operator_id: operator_id.clone(),
                                            robot_id: wire_gang_id.clone(),
                                            name: robot_name,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            };
            if let Ok(b) = encode_message(&response) {
                let _ = stream.inner.write_all(&b).await;
                let _ = stream.inner.flush().await;
            }
        })
    });

    transport
        .listen(gang_core::protocol::ProtocolId::control(), handler)
        .await
        .map_err(|e| anyhow::anyhow!("failed to register pairing handler: {e}"))?;

    // 5. Run the swarm and dial the relay so the operator becomes reachable.
    let loop_transport = Arc::clone(&transport);
    let event_loop = tokio::spawn(async move { loop_transport.run_event_loop().await });
    transport
        .dial_multiaddr(&relay_addr)
        .await
        .with_context(|| format!("dialing relay {relay_addr}"))?;
    wait_for(
        std::time::Duration::from_secs(20),
        "the operator's relay circuit reservation",
        || {
            let t = Arc::clone(&transport);
            async move {
                t.listen_addrs()
                    .await
                    .into_iter()
                    .find(|a| a.contains("p2p-circuit"))
            }
        },
    )
    .await?;

    // 6. Show the operator what to do.
    let expiry_iso =
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(token.expires_at_ms as i64)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "waiting",
                "relay_addr": relay_addr,
                "operator_id": operator_id.as_str(),
                "operator_libp2p_id": operator_libp2p_id,
                "robot_command": robot_line,
                "token": encoded,
                "expires_at": expiry_iso,
                "wait_secs": wait.as_secs(),
            }))?
        );
    } else {
        println!("=== gang pair — enroll a robot in one line ===");
        println!();
        println!("Relay:    {relay_addr}");
        println!("Operator: {operator_id}");
        println!("Expires:  {expiry_iso}");
        println!();
        println!("Run this ONE line on the robot:");
        println!();
        println!("    {robot_line}");
        println!();
        if qr {
            maybe_render_qr(true, &robot_line);
            println!();
        }
        println!(
            "Waiting up to {}s for the robot to dial out and enroll… (Ctrl-C to cancel)",
            wait.as_secs()
        );
    }

    // 7. Block until the robot enrolls, the wait elapses, or Ctrl-C.
    let outcome = tokio::select! {
        biased;
        recv = done_rx.recv() => recv,
        _ = tokio::time::sleep(wait) => None,
        _ = tokio::signal::ctrl_c() => {
            if !json_output { println!("\nPairing cancelled."); }
            let _ = TransportAdapter::shutdown(transport.as_ref()).await;
            event_loop.abort();
            anyhow::bail!("pairing cancelled before a robot enrolled");
        }
    };

    let _ = TransportAdapter::shutdown(transport.as_ref()).await;
    event_loop.abort();

    match outcome {
        Some((robot_name, robot_id, libp2p_id)) => {
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "paired",
                        "name": robot_name,
                        "robot_id": robot_id.as_str(),
                        "libp2p_id": libp2p_id,
                        "relay_addr": relay_addr,
                    }))?
                );
            } else {
                println!();
                println!("  ✔ paired: {robot_name}  ({robot_id})");
                println!();
                println!("The robot is now in your fleet. Drive it:");
                println!("  gang deploy {robot_name} <signed.wasm>");
                println!("  gang run {robot_name} <capability>");
                println!("  gang peer list");
            }
            Ok(())
        }
        None => {
            anyhow::bail!(
                "no robot enrolled within {}s. The token has not been consumed; \
                 re-run `gang pair` to mint a fresh one.",
                wait.as_secs()
            );
        }
    }
}

/// Record a freshly-paired robot in the operator's peer registry and pre-provision
/// its host key in the operator trust store, both keyed on the WIRE-AUTHENTICATED
/// identity. Called from inside the pairing handler.
fn record_paired_robot(
    registry_path: &Path,
    trust_path: &Path,
    name: &str,
    gang_id: &gang_core::identity::PeerId,
    libp2p_id: &str,
    ed25519_pubkey: &[u8; 32],
    relay_addr: &str,
) -> anyhow::Result<()> {
    use gang_core::identity::{PeerEntry, PeerRegistry, Role};
    use gang_core::manifest::{TrustStore, TrustedPeer};

    let mut registry = PeerRegistry::load(registry_path)?;
    registry.register(
        name.to_string(),
        PeerEntry {
            peer_id: gang_id.clone(),
            role: Role::RobotAgent,
            relay_addrs: vec![relay_addr.to_string()],
            libp2p_id: Some(libp2p_id.to_string()),
        },
    );
    registry.save(registry_path)?;

    // Pre-provision the robot's host key so subsequent `gang deploy` connects
    // under the default "strict" policy without a TOFU prompt — safe because the
    // key was just cryptographically authenticated during enrollment.
    let mut trust = TrustStore::load(trust_path)?;
    trust.add(TrustedPeer {
        peer_id: gang_id.clone(),
        name: name.to_string(),
        public_key: ed25519_pubkey.to_vec(),
    });
    trust.save(trust_path)?;
    Ok(())
}

/// `gang join` — the ONE line run on the robot. Decodes the token, dials out,
/// reserves a circuit, enrolls with the operator, then (unless `--once`) stays
/// online serving the control protocol so the operator can deploy.
pub async fn join(
    token_str: &str,
    name: Option<&str>,
    once: bool,
    timeout: Option<u64>,
    json_flag: bool,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::manifest::{TrustStore, TrustedPeer};
    use gang_core::message::{ControlMessage, decode_message, encode_message};
    use gang_core::pairing::PairingToken;
    use gang_core::transport::TransportAdapter;
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;
    use std::sync::Arc;

    let json_output = json_flag || matches!(format, OutputFormat::Json);
    let budget = std::time::Duration::from_secs(timeout.unwrap_or(JOIN_TIMEOUT_SECS));

    // 1. Decode and pre-check the token (friendly error before any network I/O).
    let token = PairingToken::decode(token_str)
        .map_err(|e| anyhow::anyhow!("invalid pairing token: {e}"))?;
    let now_ms = gang_core::message::unix_millis_now();
    if token.is_expired(now_ms) {
        anyhow::bail!("this pairing token has expired. Ask the operator to run `gang pair` again.");
    }
    let operator = gang_libp2p::identity_from_libp2p_str(&token.operator_libp2p_id)
        .ok_or_else(|| anyhow::anyhow!("pairing token names an invalid operator id"))?;

    // 2. Robot identity + data dir (honors global --data-dir via GANG_HOME).
    let data_dir = gang_core::identity::default_config_dir();
    std::fs::create_dir_all(&data_dir)?;
    let key_path = data_dir.join("identity.key");
    let robot_gang_id = gang_core::identity::Keypair::load_or_generate(&key_path)?.peer_id();

    // 3. Trust the operator so its later deploys are authorized (SEC-03): the
    //    operator's key is the one the token names and libp2p will authenticate.
    let trust_path = data_dir.join("trusted_peers.json");
    let mut trust = TrustStore::load(&trust_path)?;
    trust.add(TrustedPeer {
        peer_id: operator.gang_id.clone(),
        name: "pair-operator".into(),
        public_key: operator.ed25519_pubkey.to_vec(),
    });
    trust.save(&trust_path)?;

    // 4. Agent + transport that reserves a circuit on the relay. The agent runs
    //    with a permissive dev policy (policy_path: None), matching `gang agent`;
    //    a production robot would ship a default-deny policy.toml.
    let agent = Arc::new(RobotAgent::new(AgentConfig {
        key_path: key_path.clone(),
        policy_path: None,
        trust_store_path: trust_path.clone(),
        capabilities_dir: data_dir.join("capabilities"),
        audit_log_path: data_dir.join("audit.log"),
        audit_max_size_bytes: 50 * 1024 * 1024,
        fs_allowed_patterns: vec![FsRule {
            pattern: format!("{}/**", data_dir.display()),
            read: true,
            write: true,
        }],
        log_allowed_sources: vec!["**".into()],
    })?);

    let transport = Arc::new(
        gang_libp2p::Libp2pTransportAdapter::new(gang_libp2p::Libp2pConfig {
            key_path: key_path.clone(),
            listen_addrs: vec![],
            relay_addrs: vec![token.relay_addr.clone()],
            ..Default::default()
        })
        .await
        .context("building the robot transport")?,
    );
    let robot_libp2p_id = transport.libp2p_peer_id().to_string();

    agent
        .serve(transport.as_ref())
        .await
        .context("agent failed to serve the control protocol")?;
    let loop_transport = Arc::clone(&transport);
    let event_loop = tokio::spawn(async move { loop_transport.run_event_loop().await });

    if !json_output {
        println!("Joining fleet via {}…", token.relay_addr);
    }

    // 5. Dial the relay, reserve our own circuit, then dial the operator.
    let enroll = async {
        transport
            .dial_multiaddr(&token.relay_addr)
            .await
            .with_context(|| format!("dialing relay {}", token.relay_addr))?;
        wait_for(
            std::time::Duration::from_secs(20),
            "the robot's relay circuit reservation",
            || {
                let t = Arc::clone(&transport);
                async move {
                    t.listen_addrs()
                        .await
                        .into_iter()
                        .find(|a| a.contains("p2p-circuit"))
                }
            },
        )
        .await?;

        let circuit = circuit_addr(&token.relay_addr, &operator.libp2p_id);
        transport
            .dial_multiaddr(&circuit)
            .await
            .with_context(|| format!("dialing operator via circuit {circuit}"))?;

        // Wait until the operator (its gang id) is a connected, authenticated
        // peer — this is where libp2p proves we reached the operator the token
        // named, before we hand over the bearer secret.
        let redial = std::time::Duration::from_secs(2);
        let mut last = std::time::Instant::now();
        wait_for(
            std::time::Duration::from_secs(25),
            "an authenticated connection to the operator",
            || {
                let t = Arc::clone(&transport);
                let op = operator.gang_id.clone();
                let circuit = circuit.clone();
                let redial_due = last.elapsed() >= redial;
                if redial_due {
                    last = std::time::Instant::now();
                }
                async move {
                    if redial_due {
                        let _ = t.dial_multiaddr(&circuit).await;
                    }
                    t.connected_peers()
                        .await
                        .into_iter()
                        .find(|(id, _)| *id == op)
                        .map(|_| ())
                }
            },
        )
        .await?;

        let req = ControlMessage::Enroll {
            token_secret: token.secret.to_vec(),
            name: name.map(String::from).unwrap_or_default(),
            libp2p_id: robot_libp2p_id.clone(),
        };
        let req_bytes =
            encode_message(&req).map_err(|e| anyhow::anyhow!("encoding enroll: {e}"))?;
        let resp_bytes = transport
            .send_rpc_with_timeout(&operator.gang_id, req_bytes, budget)
            .await
            .map_err(|e| anyhow::anyhow!("enrollment request failed: {e}"))?;
        if resp_bytes.is_empty() {
            anyhow::bail!("operator sent no enrollment response");
        }
        let (resp, _) = decode_message::<ControlMessage>(&resp_bytes)
            .map_err(|e| anyhow::anyhow!("decoding enrollment response: {e}"))?;
        match resp {
            ControlMessage::Enrolled {
                operator_id, name, ..
            } => {
                // Defence in depth: the operator that answered must be the one
                // the token named (libp2p already enforced this on the wire).
                if operator_id != operator.gang_id {
                    anyhow::bail!(
                        "operator identity mismatch: token named {} but {} answered",
                        operator.gang_id,
                        operator_id
                    );
                }
                Ok(name)
            }
            ControlMessage::Error { code, message, .. } => {
                anyhow::bail!("operator rejected enrollment [{code}]: {message}")
            }
            _ => anyhow::bail!("unexpected enrollment response from operator"),
        }
    };

    let registered_name = match tokio::time::timeout(budget, enroll).await {
        Ok(r) => r,
        Err(_) => {
            let _ = TransportAdapter::shutdown(transport.as_ref()).await;
            event_loop.abort();
            anyhow::bail!(
                "timed out after {}s enrolling with the operator (is `gang pair` still \
                 waiting on the operator machine?)",
                budget.as_secs()
            );
        }
    };

    let registered_name = match registered_name {
        Ok(n) => n,
        Err(e) => {
            let _ = TransportAdapter::shutdown(transport.as_ref()).await;
            event_loop.abort();
            return Err(e);
        }
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "joined",
                "name": registered_name,
                "robot_id": robot_gang_id.as_str(),
                "libp2p_id": robot_libp2p_id,
                "operator_id": operator.gang_id.as_str(),
                "relay_addr": token.relay_addr,
                "serving": !once,
            }))?
        );
    } else {
        println!();
        println!(
            "  ✔ joined: registered with operator {} as '{registered_name}'",
            operator.gang_id
        );
        println!("    this robot: {robot_gang_id}");
    }

    if once {
        let _ = TransportAdapter::shutdown(transport.as_ref()).await;
        event_loop.abort();
        return Ok(());
    }

    // 6. Stay online as the agent so the operator can deploy immediately.
    if !json_output {
        println!();
        println!("Serving on the relay circuit. Press Ctrl-C to stop.");
    }
    let mut event_loop = event_loop;
    tokio::select! {
        r = &mut event_loop => {
            if let Ok(Err(e)) = r { eprintln!("transport event loop error: {e}"); }
        }
        _ = tokio::signal::ctrl_c() => {
            if !json_output { println!("\nRobot stopped."); }
        }
    }
    event_loop.abort();
    Ok(())
}

/// `gang test-archetype`
pub async fn test_archetype(archetype: &str) -> anyhow::Result<()> {
    let valid = [
        "open-warehouse",
        "nat-office",
        "enterprise-dmz",
        "mobile-cgnat",
    ];
    if !valid.contains(&archetype) {
        anyhow::bail!(
            "Unknown archetype: {archetype}\nValid archetypes: {}",
            valid.join(", ")
        );
    }

    // Check Docker is available
    let docker_check = std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match docker_check {
        Ok(s) if s.success() => {}
        _ => {
            anyhow::bail!(
                "Docker is required for test-archetype but is not available.\n\
                 Install Docker and try again."
            );
        }
    }

    // Check docker compose is available
    let compose_check = std::process::Command::new("docker")
        .args(["compose", "version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match compose_check {
        Ok(s) if s.success() => {}
        _ => {
            anyhow::bail!(
                "docker compose is required but not available.\n\
                 Install the Docker Compose plugin and try again."
            );
        }
    }

    println!("============================================");
    println!("  Ganglion Test Harness: {archetype}");
    println!("============================================");
    println!();

    // Describe what this archetype simulates
    match archetype {
        "open-warehouse" => {
            println!("Scenario: Flat L2, no NAT, permissive DHCP");
            println!("  - Direct TCP/QUIC connection between operator and robot");
            println!("  - Multicast works, no relay needed");
        }
        "nat-office" => {
            println!("Scenario: Single consumer NAT, no inbound ports");
            println!("  - Robot dials out to relay");
            println!("  - Operator connects via relay, DCUtR upgrade attempted");
        }
        "enterprise-dmz" => {
            println!("Scenario: VLAN isolation, restricted outbound ports");
            println!("  - TLS inspection proxy, TCP 443 outbound only");
            println!("  - Robot connects through firewall to relay");
        }
        "mobile-cgnat" => {
            println!("Scenario: Symmetric NAT, CGNAT, IP rotation");
            println!("  - Relay-only connectivity (DCUtR fails on symmetric NAT)");
            println!("  - Simulated cellular conditions: jitter, packet loss");
        }
        _ => unreachable!(),
    }
    println!();

    // Locate the test-harness directory relative to the binary or CWD.
    let scenario_dir = find_scenario_dir(archetype)?;
    let compose_file = scenario_dir.join("docker-compose.yml");

    if !compose_file.exists() {
        anyhow::bail!(
            "docker-compose.yml not found at {}\n\
             Make sure you're running from the Ganglion repo root.",
            compose_file.display()
        );
    }

    let project_name = format!("ganglion-{archetype}");
    let compose_path = compose_file.to_string_lossy().to_string();

    // Tear down any leftover from previous runs
    let _ = std::process::Command::new("docker")
        .args([
            "compose",
            "-p",
            &project_name,
            "-f",
            &compose_path,
            "down",
            "-v",
            "--remove-orphans",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    // Build
    println!("Building container images...");
    let build_status = std::process::Command::new("docker")
        .args(["compose", "-p", &project_name, "-f", &compose_path, "build"])
        .status()?;

    if !build_status.success() {
        anyhow::bail!("Docker build failed. Check output above.");
    }

    // Start
    println!();
    println!("Starting {archetype} scenario...");
    let up_status = std::process::Command::new("docker")
        .args([
            "compose",
            "-p",
            &project_name,
            "-f",
            &compose_path,
            "up",
            "-d",
        ])
        .status()?;

    if !up_status.success() {
        // Clean up on failure
        let _ = std::process::Command::new("docker")
            .args([
                "compose",
                "-p",
                &project_name,
                "-f",
                &compose_path,
                "down",
                "-v",
                "--remove-orphans",
            ])
            .status();
        anyhow::bail!("Failed to start scenario. Check output above.");
    }

    // Wait for stabilization: poll container state (mirrors run-tests.sh)
    // instead of a fixed sleep, bounded at ~30s.
    println!("Waiting for services to stabilize...");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let (running, total) = compose_service_states(&project_name, &compose_path);
        if total > 0 && running == total {
            break;
        }
        if std::time::Instant::now() >= deadline {
            eprintln!(
                "warning: only {running}/{total} services reached 'running' within 30s; \
                 continuing with checks."
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    // Show service status
    println!();
    let _ = std::process::Command::new("docker")
        .args(["compose", "-p", &project_name, "-f", &compose_path, "ps"])
        .status();

    // Run connectivity checks
    println!();
    println!("=== Connectivity checks ===");
    run_archetype_checks(archetype, &project_name, &compose_path);

    // Show logs
    println!();
    println!("=== Service logs (last 20 lines) ===");
    let _ = std::process::Command::new("docker")
        .args([
            "compose",
            "-p",
            &project_name,
            "-f",
            &compose_path,
            "logs",
            "--tail",
            "20",
        ])
        .status();

    println!();
    println!("============================================");
    println!("  Scenario {archetype} is running");
    println!("============================================");
    println!();
    println!("Inspect manually:");
    println!("  docker compose -p {project_name} -f {compose_path} exec robot bash");
    println!("  docker compose -p {project_name} -f {compose_path} logs -f");
    println!();
    println!("Tear down:");
    println!("  docker compose -p {project_name} -f {compose_path} down -v");

    Ok(())
}

/// Query `docker compose ps` for the scenario's container states, returning
/// `(running, total)`. Errors (docker missing, compose not up yet) count as
/// `(0, 0)` so the caller's poll loop just keeps waiting until its deadline.
fn compose_service_states(project_name: &str, compose_path: &str) -> (usize, usize) {
    let output = std::process::Command::new("docker")
        .args([
            "compose",
            "-p",
            project_name,
            "-f",
            compose_path,
            "ps",
            "-a",
            "--format",
            "{{.State}}",
        ])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let states: Vec<&str> = stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            let running = states.iter().filter(|s| **s == "running").count();
            (running, states.len())
        }
        _ => (0, 0),
    }
}

/// Run archetype-specific network connectivity checks.
fn run_archetype_checks(archetype: &str, project_name: &str, compose_path: &str) {
    let docker_exec = |service: &str, cmd: &[&str]| -> bool {
        let mut args = vec![
            "compose",
            "-p",
            project_name,
            "-f",
            compose_path,
            "exec",
            "-T",
            service,
        ];
        args.extend_from_slice(cmd);
        std::process::Command::new("docker")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };

    match archetype {
        "open-warehouse" => {
            let ok = docker_exec("operator", &["ping", "-c", "2", "-W", "2", "172.20.0.20"]);
            println!(
                "  operator -> robot (direct):  {}",
                if ok { "OK" } else { "FAIL" }
            );
            let ok = docker_exec("robot", &["ping", "-c", "2", "-W", "2", "172.20.0.10"]);
            println!(
                "  robot -> relay (direct):     {}",
                if ok { "OK" } else { "FAIL" }
            );
        }
        "nat-office" => {
            let ok = docker_exec("robot", &["ping", "-c", "2", "-W", "2", "192.168.1.1"]);
            println!(
                "  robot -> NAT gateway:        {}",
                if ok { "OK" } else { "FAIL" }
            );
            let ok = docker_exec("operator", &["ping", "-c", "2", "-W", "2", "192.168.2.1"]);
            println!(
                "  operator -> NAT gateway:     {}",
                if ok { "OK" } else { "FAIL" }
            );
        }
        "enterprise-dmz" => {
            let ok = docker_exec("robot", &["ping", "-c", "2", "-W", "2", "172.16.10.1"]);
            println!(
                "  robot -> firewall:           {}",
                if ok { "OK" } else { "FAIL" }
            );
            let ok = docker_exec("operator", &["ping", "-c", "2", "-W", "2", "10.1.0.10"]);
            println!(
                "  operator -> relay (direct):  {}",
                if ok { "OK" } else { "FAIL" }
            );
        }
        "mobile-cgnat" => {
            let ok = docker_exec("robot", &["ping", "-c", "2", "-W", "2", "10.64.0.1"]);
            println!(
                "  robot -> inner NAT:          {}",
                if ok { "OK" } else { "FAIL" }
            );
            let ok = docker_exec("operator", &["ping", "-c", "2", "-W", "2", "10.2.0.10"]);
            println!(
                "  operator -> relay (direct):  {}",
                if ok { "OK" } else { "FAIL" }
            );
        }
        _ => {}
    }
}

/// Find the test-harness scenario directory by searching upward from CWD.
fn find_scenario_dir(archetype: &str) -> anyhow::Result<std::path::PathBuf> {
    let cwd = std::env::current_dir()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join("test-harness").join(archetype);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "Could not find test-harness/{archetype} directory.\n\
         Run this command from within the Ganglion repository."
    )
}

/// Pretty-print diagnostics output for human consumption.
fn print_diagnostics(val: &serde_json::Value) {
    if let Some(sys) = val.get("system_info") {
        println!("System Information:");
        if let Some(h) = sys.get("hostname").and_then(|v| v.as_str()) {
            println!("  Hostname:  {h}");
        }
        if let Some(os) = sys.get("os").and_then(|v| v.as_str()) {
            let ver = sys.get("os_version").and_then(|v| v.as_str()).unwrap_or("");
            println!("  OS:        {os} {ver}");
        }
        if let Some(arch) = sys.get("arch").and_then(|v| v.as_str()) {
            println!("  Arch:      {arch}");
        }
        if let Some(cpus) = sys.get("cpu_count").and_then(|v| v.as_u64()) {
            println!("  CPUs:      {cpus}");
        }
        if let Some(mem) = sys.get("memory_total_bytes").and_then(|v| v.as_u64())
            && mem > 0
        {
            println!("  Memory:    {} GB", mem / (1024 * 1024 * 1024));
        }
        if let Some(uptime) = sys.get("uptime_secs").and_then(|v| v.as_u64()) {
            let hours = uptime / 3600;
            let mins = (uptime % 3600) / 60;
            println!("  Uptime:    {hours}h {mins}m");
        }
        if let Some(ver) = sys.get("ganglion_version").and_then(|v| v.as_str()) {
            println!("  Ganglion:  v{ver}");
        }
        println!();
    }

    if let Some(net) = val.get("network")
        && let Some(interfaces) = net.get("interfaces").and_then(|v| v.as_array())
    {
        println!("Network Interfaces:");
        for iface in interfaces {
            let name = iface.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let up = iface
                .get("is_up")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let status = if up { "UP" } else { "DOWN" };
            let addrs = iface
                .get("addresses")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("  {name} ({status}): {addrs}");
        }
        println!();
    }

    if let Some(procs) = val.get("processes").and_then(|v| v.as_array()) {
        println!("Processes: {} running", procs.len());
        // Show top 5 by CPU
        let mut sorted: Vec<&serde_json::Value> = procs.iter().collect();
        sorted.sort_by(|a, b| {
            let cpu_a = a.get("cpu_percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cpu_b = b.get("cpu_percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
            cpu_b
                .partial_cmp(&cpu_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for proc in sorted.iter().take(5) {
            let name = proc.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let pid = proc.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
            let cpu = proc
                .get("cpu_percent")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            println!("  PID {pid}: {cpu:.1}% CPU — {name}");
        }
        println!();
    }

    if let Some(logs) = val.get("log_sources").and_then(|v| v.as_array()) {
        println!("Log Sources:");
        for source in logs {
            let name = source.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let stype = source
                .get("source_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            println!("  {name} ({stype})");
        }
    }
}

/// `gang diagnose` — detect network archetype and recommend transport config.
pub async fn diagnose(robot: Option<&str>, format: &crate::OutputFormat) -> anyhow::Result<()> {
    use gang_ros::archetype;

    if let Some(robot_name) = robot {
        println!("Diagnosing network for robot: {robot_name}");
        println!("(Remote diagnosis requires active connection — running local probes instead)");
        println!();
    }

    println!("Running network probes...");
    println!();

    let result = archetype::detect_archetype();

    match format {
        crate::OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&result)?;
            println!("{json}");
        }
        crate::OutputFormat::Text => {
            println!("============================================");
            println!("  Network Archetype Detection");
            println!("============================================");
            println!();
            println!(
                "  Detected:    {} ({:.0}% confidence)",
                result.archetype,
                result.confidence * 100.0
            );
            println!();

            println!("Probes:");
            for probe in &result.probes {
                let status = if probe.success { "✓" } else { "✗" };
                println!("  {status} {}: {}", probe.probe_name, probe.detail);
            }
            println!();

            println!("Recommendations:");
            for rec in &result.recommendations {
                println!("  → {rec}");
            }
        }
    }

    Ok(())
}

/// `gang transport-stats` — show REAL per-transport statistics for the live
/// circuit to a robot (from the operator transport's connected-peer counters).
pub async fn transport_stats(
    robot: &str,
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    timeout_secs: Option<u64>,
    format: &crate::OutputFormat,
) -> anyhow::Result<()> {
    let target = resolve_target(robot, explicit_peer, explicit_relay)?;
    if target.is_local {
        anyhow::bail!(
            "transport-stats needs a live network connection; '{robot}' resolved to a local \
             agent. Point at a remote robot (registered name, peer id, or --peer/--relay)."
        );
    }
    let remote = prepare_remote(&target)?;
    let display = remote.display();
    let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(CONTROL_TIMEOUT_SECS));

    let conn = establish_remote_connection(&remote, timeout).await?;
    // Exchange one control message so the counters reflect real traffic, then
    // read the operator's per-connection stats for this circuit.
    let _ = conn
        .transport
        .send_rpc_with_timeout(
            &remote.gang_id,
            gang_core::message::encode_message(
                &gang_core::message::ControlMessage::ListCapabilities,
            )
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?,
            timeout,
        )
        .await
        .map_err(|e| anyhow::anyhow!("probe request to '{display}' failed: {e}"))?;

    let stats = gang_core::transport::TransportAdapter::transport_stats(
        conn.transport.as_ref(),
        &remote.gang_id,
    )
    .await;
    conn.close().await;

    let stats = stats.ok_or_else(|| {
        anyhow::anyhow!("no live connection statistics available for '{display}'")
    })?;

    match format {
        crate::OutputFormat::Json => {
            let mut json = serde_json::to_value(&stats)?;
            if let Some(obj) = json.as_object_mut() {
                obj.insert("peer".into(), serde_json::json!(remote.gang_id.as_str()));
            }
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        crate::OutputFormat::Text => {
            println!("Transport statistics for '{display}' (live circuit):");
            println!("  Transport:       {}", stats.transport);
            println!("  Via relay:       {}", stats.via_relay);
            println!("  Connect time:    {}ms", stats.connect_time_ms);
            println!(
                "  Messages:        {} sent, {} received",
                stats.messages_sent, stats.messages_received
            );
            println!(
                "  Bytes:           {} sent, {} received",
                format_bytes(stats.bytes_sent),
                format_bytes(stats.bytes_received)
            );
            if let Some(rtt) = stats.last_rtt_ms {
                println!("  Last RTT:        {rtt}ms");
            }
            println!(
                "  DCUtR:           attempted={}, succeeded={}",
                stats.dcutr_attempted, stats.dcutr_succeeded
            );
            println!("  Uptime:          {}", format_duration(stats.uptime_secs));
            println!("  Reconnections:   {}", stats.reconnections);
        }
    }

    Ok(())
}

// --- Event subscription commands (logs / connect / list) ---

/// How often `--follow` / `connect` re-polls the robot's event feed. The feed
/// rides request-response (see `ControlMessage::SubscribeEvents`), so a live
/// tail is a bounded poll rather than a persistent push; a genuine push stream
/// is the reserved `/ganglion/events/1.0` direct-substream path.
pub(crate) const EVENT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Parse a short duration like `30s`, `5m`, `2h`, `1d` into a `chrono::Duration`.
fn parse_since(spec: &str) -> anyhow::Result<chrono::Duration> {
    let spec = spec.trim();
    let (num, unit) = spec.split_at(
        spec.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(spec.len()),
    );
    let value: i64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid --since '{spec}': expected e.g. 30s, 5m, 2h, 1d"))?;
    let dur = match unit {
        "s" | "" => chrono::Duration::seconds(value),
        "m" => chrono::Duration::minutes(value),
        "h" => chrono::Duration::hours(value),
        "d" => chrono::Duration::days(value),
        other => anyhow::bail!("invalid --since unit '{other}': use s, m, h, or d"),
    };
    Ok(dur)
}

/// The timestamp carried by an event, if any (snapshot/gap have none).
fn event_ts(ev: &gang_core::events::AgentEvent) -> Option<chrono::DateTime<chrono::Utc>> {
    use gang_core::events::AgentEvent::*;
    match ev {
        PolicyDecision { ts, .. } | ConnectionChanged { ts, .. } | Heartbeat { ts, .. } => {
            Some(*ts)
        }
        AuditAppended { record, .. } => Some(record.ended_at),
        PresenceSnapshot { .. } | Gap { .. } => None,
        _ => None,
    }
}

/// Whether this event is a log-relevant line (`gang logs` shows audit + policy).
fn is_log_event(ev: &gang_core::events::AgentEvent) -> bool {
    use gang_core::events::AgentEvent::*;
    matches!(
        ev,
        AuditAppended { .. } | PolicyDecision { .. } | Gap { .. }
    )
}

/// Render one event as a human-readable line.
fn event_human_line(ev: &gang_core::events::AgentEvent) -> String {
    use gang_core::events::{AgentEvent::*, ConnectionState, PolicyOutcome};
    match ev {
        PresenceSnapshot {
            ganglion_version,
            uptime_secs,
            archetype,
            installed_capabilities,
            ..
        } => format!(
            "presence  v{ganglion_version}  up {}  archetype={}  caps=[{}]",
            format_duration(*uptime_secs),
            archetype.as_deref().unwrap_or("unknown"),
            installed_capabilities.join(", ")
        ),
        PolicyDecision {
            ts,
            operator_peer,
            capability_group,
            decision,
            reason,
            ..
        } => {
            let verdict = match decision {
                PolicyOutcome::Allow => "ALLOW",
                PolicyOutcome::Deny => "DENY ",
                _ => "?????",
            };
            format!(
                "{}  policy {verdict}  {capability_group}  by {}  ({reason})",
                ts.format("%Y-%m-%dT%H:%M:%SZ"),
                short_peer(operator_peer)
            )
        }
        AuditAppended { record, .. } => format!(
            "{}  audit  {} v{}  by {}  -> {}  caps=[{}]",
            record.ended_at.format("%Y-%m-%dT%H:%M:%SZ"),
            record.component_name,
            record.component_version,
            short_peer(&record.operator_peer),
            record.exit,
            record.capabilities_used.join(", ")
        ),
        ConnectionChanged {
            ts,
            peer,
            transport,
            via_relay,
            state,
            ..
        } => {
            let dir = match state {
                ConnectionState::Up => "UP  ",
                ConnectionState::Down => "DOWN",
                _ => "????",
            };
            format!(
                "{}  conn {dir}  {}  transport={transport} via_relay={via_relay}",
                ts.format("%Y-%m-%dT%H:%M:%SZ"),
                short_peer(peer)
            )
        }
        Heartbeat {
            ts, uptime_secs, ..
        } => format!(
            "{}  heartbeat  up {}",
            ts.format("%Y-%m-%dT%H:%M:%SZ"),
            format_duration(*uptime_secs)
        ),
        Gap { dropped } => format!("--- gap: {dropped} event(s) dropped (fell behind) ---"),
        _ => "unknown event".to_string(),
    }
}

/// Abbreviate a peer id for a compact log line.
fn short_peer(p: &gang_core::identity::PeerId) -> String {
    let s = p.as_str();
    if s.len() > 13 {
        format!("{}…", &s[..13])
    } else {
        s.to_string()
    }
}

/// `gang logs <robot> [--follow] [--since <dur>]` — print AuditAppended (+
/// PolicyDecision) events. Without `--follow`, prints the recent context from
/// the presence snapshot and exits; with `--follow`, tails live.
pub async fn logs(
    robot: &str,
    follow: bool,
    since: Option<&str>,
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    let cutoff = match since {
        Some(spec) => Some(chrono::Utc::now() - parse_since(spec)?),
        None => None,
    };

    let target = resolve_target(robot, explicit_peer, explicit_relay)?;
    if target.is_local {
        anyhow::bail!(
            "`gang logs` needs a live connection; '{robot}' resolved to a local agent. \
             Point at a remote robot (name, peer id, or --peer/--relay)."
        );
    }
    let remote = prepare_remote(&target)?;
    let display = remote.display();
    let timeout = std::time::Duration::from_secs(CONTROL_TIMEOUT_SECS);
    let conn = establish_remote_connection(&remote, timeout).await?;

    let print = |ev: &gang_core::events::AgentEvent| {
        if let Some(cut) = cutoff
            && let Some(ts) = event_ts(ev)
            && ts < cut
        {
            return;
        }
        match format {
            OutputFormat::Json => {
                if let Ok(line) = serde_json::to_string(ev) {
                    println!("{line}");
                }
            }
            OutputFormat::Text => println!("{}", event_human_line(ev)),
        }
    };

    // Initial fresh subscription: presence snapshot + recent context.
    let batch = conn
        .transport
        .subscribe_events(&remote.gang_id, None, timeout)
        .await
        .map_err(|e| anyhow::anyhow!("subscribing to '{display}' failed: {e}"))?;

    let mut cursor: u64 = 0;
    for ev in &batch {
        if let Some(s) = ev.seq() {
            cursor = cursor.max(match ev {
                gang_core::events::AgentEvent::PresenceSnapshot { .. } => s,
                _ => s + 1,
            });
        }
        if is_log_event(ev) {
            print(ev);
        }
    }

    if !follow {
        conn.close().await;
        return Ok(());
    }

    if let OutputFormat::Text = format {
        eprintln!("--- following '{display}' (Ctrl-C to stop) ---");
    }
    let result = follow_events(&conn, &remote, cursor, timeout, is_log_event, &print).await;
    conn.close().await;
    result
}

/// `gang connect <robot>` — attach a live status view (presence + heartbeat +
/// connection state + a live tail of policy/audit) as scrolling text until
/// Ctrl-C. The non-TUI precursor to the dashboard; reuses the subscription API.
pub async fn connect(robot: &str, format: &OutputFormat) -> anyhow::Result<()> {
    let target = resolve_target(robot, None, None)?;
    if target.is_local {
        anyhow::bail!(
            "`gang connect` needs a live connection; '{robot}' resolved to a local agent. \
             Point at a remote robot (name, peer id)."
        );
    }
    let remote = prepare_remote(&target)?;
    let display = remote.display();
    let timeout = std::time::Duration::from_secs(CONTROL_TIMEOUT_SECS);
    let conn = establish_remote_connection(&remote, timeout).await?;

    let print = |ev: &gang_core::events::AgentEvent| match format {
        OutputFormat::Json => {
            if let Ok(line) = serde_json::to_string(ev) {
                println!("{line}");
            }
        }
        OutputFormat::Text => println!("{}", event_human_line(ev)),
    };

    if let OutputFormat::Text = format {
        println!("Connected to '{display}'. Live status (Ctrl-C to detach):");
    }

    let batch = conn
        .transport
        .subscribe_events(&remote.gang_id, None, timeout)
        .await
        .map_err(|e| anyhow::anyhow!("connecting to '{display}' failed: {e}"))?;

    let mut cursor: u64 = 0;
    for ev in &batch {
        if let Some(s) = ev.seq() {
            cursor = cursor.max(match ev {
                gang_core::events::AgentEvent::PresenceSnapshot { .. } => s,
                _ => s + 1,
            });
        }
        print(ev);
    }

    // Show everything in the live view.
    let all = |_ev: &gang_core::events::AgentEvent| true;
    let result = follow_events(&conn, &remote, cursor, timeout, all, &print).await;
    conn.close().await;
    result
}

/// Poll the robot's event feed, printing new events until Ctrl-C. `keep`
/// filters which events to print; `cursor` is the next-expected sequence.
async fn follow_events(
    conn: &RemoteConnection,
    remote: &RemoteTarget,
    mut cursor: u64,
    timeout: std::time::Duration,
    keep: impl Fn(&gang_core::events::AgentEvent) -> bool,
    print: &impl Fn(&gang_core::events::AgentEvent),
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(EVENT_POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                return Ok(());
            }
            _ = ticker.tick() => {
                // A cursor of 0 means "nothing seen yet" — re-request fresh.
                let since = cursor.checked_sub(1);
                let batch = match conn
                    .transport
                    .subscribe_events(&remote.gang_id, since, timeout)
                    .await
                {
                    Ok(b) => b,
                    // Transient failure while tailing: report on stderr, keep going.
                    Err(e) => {
                        eprintln!("warning: event poll failed: {e}");
                        continue;
                    }
                };
                for ev in &batch {
                    // A fresh re-request (since == None) re-sends the snapshot;
                    // skip re-printing it while following.
                    if since.is_none()
                        && matches!(ev, gang_core::events::AgentEvent::PresenceSnapshot { .. })
                    {
                        if let Some(s) = ev.seq() {
                            cursor = cursor.max(s);
                        }
                        continue;
                    }
                    if let Some(s) = ev.seq() {
                        cursor = cursor.max(s + 1);
                    }
                    if keep(ev) {
                        print(ev);
                    }
                }
            }
        }
    }
}

/// `gang list` — list registered robot-agent peers with live reachability from
/// a quick presence probe over each peer's relay circuit.
pub async fn list(format: &OutputFormat) -> anyhow::Result<()> {
    use gang_core::identity::{PeerRegistry, Role, default_registry_path};

    let registry = PeerRegistry::load(&default_registry_path()).unwrap_or_default();
    let robots: Vec<(String, gang_core::identity::PeerEntry)> = registry
        .list()
        .filter(|(_, e)| matches!(e.role, Role::RobotAgent))
        .map(|(n, e)| (n.to_string(), e.clone()))
        .collect();

    if robots.is_empty() {
        match format {
            OutputFormat::Json => println!("[]"),
            OutputFormat::Text => println!(
                "No robot-agent peers registered. Add one with `gang peer add <name> <id> --relay <multiaddr>`."
            ),
        }
        return Ok(());
    }

    // Probe each robot with a short per-peer timeout, reusing a single operator
    // transport. A probe = connect over the circuit + a fresh presence subscribe.
    let probe_timeout = std::time::Duration::from_secs(10);
    let mut rows: Vec<serde_json::Value> = Vec::new();

    for (name, entry) in &robots {
        let target = ResolvedTarget {
            peer_id: Some(entry.peer_id.clone()),
            libp2p_id: entry.libp2p_id.clone(),
            relay_addr: entry.relay_addrs.first().cloned(),
            name: Some(name.clone()),
            is_local: false,
        };
        let (reachable, detail) = match prepare_remote(&target) {
            Ok(remote) => match probe_presence(&remote, probe_timeout).await {
                Ok(Some((version, uptime))) => {
                    (true, format!("v{version}, up {}", format_duration(uptime)))
                }
                Ok(None) => (false, "no presence snapshot".to_string()),
                Err(e) => (false, format!("unreachable: {e}")),
            },
            Err(e) => (false, format!("not dispatchable: {e}")),
        };

        match format {
            OutputFormat::Json => rows.push(serde_json::json!({
                "name": name,
                "peer_id": entry.peer_id.as_str(),
                "reachable": reachable,
                "detail": detail,
            })),
            OutputFormat::Text => {
                let mark = if reachable { "up  " } else { "down" };
                println!("  [{mark}] {name}  {}  {detail}", entry.peer_id);
            }
        }
    }

    if let OutputFormat::Json = format {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    }
    Ok(())
}

/// Connect to a robot and fetch its presence snapshot; returns the version and
/// uptime when reachable and authorized.
async fn probe_presence(
    remote: &RemoteTarget,
    timeout: std::time::Duration,
) -> anyhow::Result<Option<(String, u64)>> {
    let conn = establish_remote_connection(remote, timeout).await?;
    let batch = conn
        .transport
        .subscribe_events(&remote.gang_id, None, timeout)
        .await;
    conn.close().await;

    let batch = batch.map_err(|e| anyhow::anyhow!("{e}"))?;
    for ev in batch {
        if let gang_core::events::AgentEvent::PresenceSnapshot {
            ganglion_version,
            uptime_secs,
            ..
        } = ev
        {
            return Ok(Some((ganglion_version, uptime_secs)));
        }
    }
    Ok(None)
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

/// `gang fetch <cid>` — retrieve an artifact by CID.
pub async fn fetch_artifact(
    cid_str: &str,
    output: Option<&str>,
    format: &crate::OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::artifacts::{ArtifactStore, ArtifactStoreConfig, Cid};

    let store_dir = artifact_store_dir();
    let mut store = ArtifactStore::open(ArtifactStoreConfig {
        store_dir,
        ..Default::default()
    })?;

    let cid = Cid::parse(cid_str).with_context(|| format!("invalid CID '{cid_str}'"))?;
    if !store.contains(&cid) {
        anyhow::bail!(
            "Artifact {cid_str} not found in local store.\n\
             Remote fetch from peers is not yet implemented."
        );
    }

    let data = store.retrieve(&cid)?;
    let meta = store.meta(&cid);

    let dest = match output {
        Some(path) => path.to_string(),
        None => meta
            .and_then(|m| m.filename.as_deref())
            .unwrap_or("artifact.bin")
            .to_string(),
    };
    std::fs::write(&dest, &data).with_context(|| format!("writing {dest}"))?;

    match format {
        crate::OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cid": cid.as_str(),
                    "path": dest,
                    "bytes": data.len(),
                }))?
            );
        }
        crate::OutputFormat::Text => {
            println!("Wrote {} bytes to {dest}", data.len());
        }
    }

    Ok(())
}

/// `gang push <path>` — publish a local file to the content store.
pub async fn push_artifact(
    path: &str,
    content_type: Option<&str>,
    format: &crate::OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::artifacts::{ArtifactStore, ArtifactStoreConfig};

    let store_dir = artifact_store_dir();
    let mut store = ArtifactStore::open(ArtifactStoreConfig {
        store_dir,
        ..Default::default()
    })?;

    let data = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    let filename = Path::new(path).file_name().and_then(|n| n.to_str());

    let cid = store.store(&data, filename, None, content_type)?;

    match format {
        crate::OutputFormat::Json => {
            let info = serde_json::json!({
                "cid": cid.as_str(),
                "size": data.len(),
                "filename": filename,
            });
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        crate::OutputFormat::Text => {
            println!("Published artifact:");
            println!("  CID:      {cid}");
            println!("  Size:     {}", format_bytes(data.len() as u64));
            if let Some(name) = filename {
                println!("  Filename: {name}");
            }
        }
    }

    Ok(())
}

/// `gang artifacts` — list locally-stored artifacts.
pub async fn list_artifacts(format: &crate::OutputFormat) -> anyhow::Result<()> {
    use gang_core::artifacts::{ArtifactStore, ArtifactStoreConfig};

    let store_dir = artifact_store_dir();
    let store = ArtifactStore::open(ArtifactStoreConfig {
        store_dir,
        ..Default::default()
    })?;

    let artifacts = store.list();

    match format {
        crate::OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&artifacts)?;
            println!("{json}");
        }
        crate::OutputFormat::Text => {
            if artifacts.is_empty() {
                println!("No artifacts stored locally.");
            } else {
                println!(
                    "Stored artifacts ({}, {}):",
                    artifacts.len(),
                    format_bytes(store.total_bytes())
                );
                println!();
                for meta in &artifacts {
                    let name = meta.filename.as_deref().unwrap_or("(unnamed)");
                    let chunks = if meta.chunk_count > 1 {
                        format!(" ({} chunks)", meta.chunk_count)
                    } else {
                        String::new()
                    };
                    println!("  {} — {}{}", meta.cid, format_bytes(meta.size), chunks);
                    println!("    Filename: {name}");
                    if let Some(origin) = &meta.origin_peer {
                        println!("    Origin:   {origin}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Default artifact store directory.
fn artifact_store_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("gang")
        .join("artifacts")
}

/// `gang capability scaffold <name> --language <lang>`
pub async fn capability_scaffold(
    name: &str,
    language: &str,
    output_dir: Option<&str>,
) -> anyhow::Result<()> {
    let base = output_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let project_dir = base.join(name);

    if project_dir.exists() {
        anyhow::bail!("directory {} already exists", project_dir.display());
    }

    std::fs::create_dir_all(&project_dir)?;

    match language {
        "rust" => scaffold_rust(name, &project_dir)?,
        "cpp" | "c++" => scaffold_cpp(name, &project_dir)?,
        "python" | "py" => scaffold_python(name, &project_dir)?,
        "go" | "golang" => scaffold_go(name, &project_dir)?,
        _ => anyhow::bail!("unsupported language: {language}. Supported: rust, cpp, python, go"),
    }

    // Write the real WIT interface into the project. Embedded at build time
    // from this crate's vendored copy (canonical copy lives in gang-wasm-host;
    // the sync test below keeps them identical). A path outside the crate would
    // break `cargo package` verification.
    const GANGLION_WIT: &str = include_str!("../wit/ganglion.wit");
    let wit_dir = project_dir.join("wit");
    std::fs::create_dir_all(&wit_dir)?;
    std::fs::write(wit_dir.join("ganglion.wit"), GANGLION_WIT)?;

    println!(
        "Scaffolded {} capability at {}",
        language,
        project_dir.display()
    );
    println!("\nNext steps:");
    println!("  1. Implement your capability logic (WIT is in {name}/wit/ganglion.wit)");
    println!("  2. Build: see docs/CAPABILITY_AUTHOR_GUIDE.md");
    println!("  3. Sign: gang sign {name}.component.wasm --name {name} --version 0.1.0");
    Ok(())
}

fn scaffold_rust(name: &str, dir: &Path) -> anyhow::Result<()> {
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    let crate_name = name.replace('-', "_");

    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
        ),
    )?;

    std::fs::write(
        src_dir.join("lib.rs"),
        format!(
            r#"//! {name} — a Ganglion capability.
//!
//! Build: cargo build --target wasm32-wasip2 --release
//! Component: wasm-tools component new target/wasm32-wasip2/release/{crate_name}.wasm -o {name}.component.wasm
//! Sign: gang sign {name}.component.wasm --name {name} --version 0.1.0

use serde::Serialize;

#[derive(Serialize)]
struct Output {{
    status: String,
    message: String,
}}

/// Entry point called by the Ganglion runtime.
pub fn run(args: Vec<String>) -> Result<Vec<u8>, String> {{
    let output = Output {{
        status: "ok".into(),
        message: format!("{name} invoked with {{}} arg(s)", args.len()),
    }};
    serde_json::to_vec(&output).map_err(|e| e.to_string())
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn run_returns_ok() {{
        let result = run(vec!["test".into()]).unwrap();
        let output: Output = serde_json::from_slice(&result).unwrap();
        assert_eq!(output.status, "ok");
    }}
}}
"#
        ),
    )?;

    std::fs::write(
        dir.join("Makefile"),
        format!(
            r#".PHONY: build component sign clean

build:
	cargo build --target wasm32-wasip2 --release

component: build
	wasm-tools component new target/wasm32-wasip2/release/{crate_name}.wasm \
		-o {name}.component.wasm

sign: component
	gang sign {name}.component.wasm --name {name} --version 0.1.0

test:
	cargo test

clean:
	cargo clean
	rm -f {name}.component.wasm {name}.manifest.cbor
"#
        ),
    )?;

    Ok(())
}

fn scaffold_cpp(name: &str, dir: &Path) -> anyhow::Result<()> {
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    std::fs::write(
        src_dir.join("main.cpp"),
        format!(
            r#"// {name} — a Ganglion capability (C++)
//
// Build with wasi-sdk:
//   make component

#include <cstdio>
#include <cstring>

// Entry point — called by the Ganglion runtime
extern "C" int run(int argc, const char* argv[]) {{
    printf("{{\\"status\\":\\"ok\\",\\"message\\":\\"{name} invoked with %d arg(s)\\"}}\\n", argc);
    return 0;
}}
"#
        ),
    )?;

    std::fs::write(
        dir.join("Makefile"),
        format!(
            r#"WASI_SDK ?= $(WASI_SDK_PATH)
CC = $(WASI_SDK)/bin/clang++

.PHONY: build component sign clean

build: src/main.cpp
	$(CC) -o {name}.wasm src/main.cpp --target=wasm32-wasip2 -O2

component: build
	wasm-tools component new {name}.wasm -o {name}.component.wasm

sign: component
	gang sign {name}.component.wasm --name {name} --version 0.1.0

clean:
	rm -f {name}.wasm {name}.component.wasm {name}.manifest.cbor
"#
        ),
    )?;

    Ok(())
}

fn scaffold_python(name: &str, dir: &Path) -> anyhow::Result<()> {
    std::fs::write(
        dir.join("app.py"),
        format!(
            r#"\"\"\"
{name} — a Ganglion capability (Python).

Build: componentize-py -d wit/ganglion.wit -w ganglion-capability componentize app -o {name}.component.wasm
Sign:  gang sign {name}.component.wasm --name {name} --version 0.1.0
\"\"\"

import json


def run(args: list[str]) -> bytes:
    \"\"\"Entry point called by the Ganglion runtime.\"\"\"
    result = {{
        "status": "ok",
        "message": f"{name} invoked with {{len(args)}} arg(s)",
        "args": args,
    }}
    return json.dumps(result).encode()
"#
        ),
    )?;

    std::fs::write(
        dir.join("Makefile"),
        format!(
            r#".PHONY: component sign clean

component:
	componentize-py -d wit/ganglion.wit -w ganglion-capability componentize app -o {name}.component.wasm

sign: component
	gang sign {name}.component.wasm --name {name} --version 0.1.0

clean:
	rm -f {name}.component.wasm {name}.manifest.cbor
"#
        ),
    )?;

    Ok(())
}

fn scaffold_go(name: &str, dir: &Path) -> anyhow::Result<()> {
    let mod_name = name.replace('-', "");

    std::fs::write(
        dir.join("main.go"),
        format!(
            r#"// {name} — a Ganglion capability (Go/TinyGo).
//
// Build: tinygo build -o {name}.wasm -target=wasip2 .
// Component: wasm-tools component new {name}.wasm -o {name}.component.wasm
// Sign: gang sign {name}.component.wasm --name {name} --version 0.1.0

package main

import (
	"encoding/json"
	"fmt"
	"os"
)

type Result struct {{
	Status  string `json:"status"`
	Message string `json:"message"`
}}

func main() {{
	result := Result{{
		Status:  "ok",
		Message: fmt.Sprintf("{name} invoked with %d arg(s)", len(os.Args)-1),
	}}
	data, _ := json.Marshal(result)
	fmt.Println(string(data))
}}
"#
        ),
    )?;

    std::fs::write(
        dir.join("go.mod"),
        format!("module github.com/tafy-labs/{mod_name}\n\ngo 1.22\n"),
    )?;

    std::fs::write(
        dir.join("Makefile"),
        format!(
            r#".PHONY: build component sign clean

build:
	tinygo build -o {name}.wasm -target=wasip2 .

component: build
	wasm-tools component new {name}.wasm -o {name}.component.wasm

sign: component
	gang sign {name}.component.wasm --name {name} --version 0.1.0

clean:
	rm -f {name}.wasm {name}.component.wasm {name}.manifest.cbor
"#
        ),
    )?;

    Ok(())
}

/// Default registry directory.
fn registry_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("gang")
        .join("registry")
}

/// `gang registry search <query>`
pub async fn registry_search(query: &str, format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;
    let results = reg.search(query);

    if let OutputFormat::Json = format {
        let entries: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "version": r.latest_version,
                    "description": r.description,
                    "language": r.language.to_string(),
                    "author": r.author,
                    "tags": r.tags,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No capabilities found matching \"{query}\".");
        return Ok(());
    }

    println!("Found {} result(s) for \"{}\":\n", results.len(), query);
    for r in &results {
        println!("  {} v{}", r.name, r.latest_version);
        println!("    {}", r.description);
        println!(
            "    Language: {}  Author: {}...{}",
            r.language,
            &r.author[..8.min(r.author.len())],
            &r.author[r.author.len().saturating_sub(4)..]
        );
        if !r.tags.is_empty() {
            println!("    Tags: {}", r.tags.join(", "));
        }
        println!();
    }
    Ok(())
}

/// `gang registry install <name>`
pub async fn registry_install(
    name: &str,
    version: Option<&str>,
    _format: &OutputFormat,
) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;

    let entry = if let Some(ver) = version {
        reg.get(name)
            .and_then(|versions| versions.iter().find(|e| e.version == ver))
    } else {
        reg.get_latest(name)
    };

    match entry {
        Some(entry) => {
            println!("Installing {} v{} ...", entry.name, entry.version);
            println!("  Component CID: {}", entry.component_cid);
            println!("  Manifest CID:  {}", entry.manifest_cid);
            println!("  Language:       {}", entry.language);
            // Actual fetch would use the artifact store to retrieve by CID
            println!("\nNote: network fetch not yet implemented.");
            println!(
                "Use `gang fetch {}` to retrieve the component.",
                entry.component_cid
            );
        }
        None => {
            let msg = if let Some(ver) = version {
                format!("{}@{} not found in registry.", name, ver)
            } else {
                format!("{} not found in registry.", name)
            };
            eprintln!("{msg}");
            eprintln!("Use `gang registry search` to discover available capabilities.");
        }
    }
    Ok(())
}

/// `gang registry publish <wasm_path>`
pub async fn registry_publish(
    wasm_path: &str,
    description: Option<&str>,
    tags: Option<&[String]>,
    version_override: Option<&str>,
    language_override: Option<&str>,
    _format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::manifest::SignedManifest;
    use gang_core::registry::CapabilityLanguage;

    let path = Path::new(wasm_path);
    if !path.exists() {
        anyhow::bail!("file not found: {wasm_path}");
    }

    // Read the component and compute CID
    let data = std::fs::read(path).with_context(|| format!("reading {wasm_path}"))?;
    let component_cid = gang_core::artifacts::Cid::from_bytes(&data);

    // Read the adjacent signed manifest if present — its verified contents are
    // the source of truth for name/version/language/capabilities/min-version.
    let manifest_path = path.with_extension("manifest.cbor");
    let (manifest_cid, manifest, signed_manifest) = if manifest_path.exists() {
        let manifest_bytes = std::fs::read(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let cid = gang_core::artifacts::Cid::from_bytes(&manifest_bytes);
        let signed = SignedManifest::from_cbor(&manifest_bytes)
            .with_context(|| format!("decoding manifest {}", manifest_path.display()))?;
        let decoded = signed
            .verify_and_decode()
            .with_context(|| format!("verifying manifest {}", manifest_path.display()))?;
        (cid, Some(decoded), Some(signed))
    } else {
        // No manifest found; compute CID from the component bytes as fallback
        (gang_core::artifacts::Cid::from_bytes(&data), None, None)
    };

    // Name: manifest > filename.
    let name = manifest
        .as_ref()
        .map(|m| m.name.clone())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    // Version: --version flag > manifest > default.
    let version = version_override
        .map(String::from)
        .or_else(|| manifest.as_ref().map(|m| m.version.clone()))
        .unwrap_or_else(|| "0.1.0".to_string());

    // Language: --language flag > manifest > default (Rust).
    let language = match language_override {
        Some(lang) => parse_language(lang)?,
        None => manifest
            .as_ref()
            .map(|m| m.language)
            .unwrap_or(CapabilityLanguage::Rust),
    };

    let declared_capabilities = manifest
        .as_ref()
        .map(|m| {
            m.declared_capabilities
                .iter()
                .map(|g| g.qualified_name())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let min_ganglion_version = manifest
        .as_ref()
        .and_then(|m| m.min_ganglion_version.clone());

    // Description: --description flag > manifest > default.
    let description = description
        .map(String::from)
        .or_else(|| {
            manifest
                .as_ref()
                .map(|m| m.description.clone())
                .filter(|d| !d.is_empty())
        })
        .unwrap_or_else(|| "A Ganglion capability".to_string());

    // Load identity for author
    let key_path = gang_core::identity::default_key_path();
    let author = if key_path.exists() {
        let kp = gang_core::identity::Keypair::load(&key_path)?;
        kp.peer_id().as_str().to_string()
    } else {
        "unknown".to_string()
    };

    let entry = gang_core::registry::RegistryEntry {
        name: name.clone(),
        version: version.clone(),
        description,
        author_peer_id: author,
        language,
        component_cid: component_cid.clone(),
        manifest_cid,
        declared_capabilities,
        published_at: chrono::Utc::now().to_rfc3339(),
        tags: tags.map(|t| t.to_vec()).unwrap_or_default(),
        min_ganglion_version,
    };

    // SEC-15: the registry now authenticates every entry against a signed
    // manifest. Publishing without one is no longer possible.
    let signed_manifest = signed_manifest.ok_or_else(|| {
        anyhow::anyhow!(
            "no signed manifest found next to the component ({}).\n\
             Registry entries must be authenticated: sign the component first \
             with `gang sign {wasm_path}`, then publish.",
            manifest_path.display()
        )
    })?;

    let mut reg = gang_core::registry::Registry::open(&registry_dir())?;
    reg.publish(entry, &signed_manifest)?;

    println!("Published {name} v{version} to local registry.");
    println!("  Component CID: {}", component_cid);
    println!("  Registry path: {}", registry_dir().display());
    Ok(())
}

/// Parse a language string into a `CapabilityLanguage`.
fn parse_language(lang: &str) -> anyhow::Result<gang_core::registry::CapabilityLanguage> {
    use gang_core::registry::CapabilityLanguage;
    match lang.to_lowercase().as_str() {
        "rust" | "rs" => Ok(CapabilityLanguage::Rust),
        "cpp" | "c++" => Ok(CapabilityLanguage::Cpp),
        "python" | "py" => Ok(CapabilityLanguage::Python),
        "go" | "golang" => Ok(CapabilityLanguage::Go),
        other => anyhow::bail!("unknown language '{other}'. Valid: rust, cpp, python, go"),
    }
}

/// `gang registry list`
pub async fn registry_list(format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;
    let list = reg.list();

    if let OutputFormat::Json = format {
        let entries: Vec<_> = list
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "version": r.latest_version,
                    "description": r.description,
                    "language": r.language.to_string(),
                    "author": r.author,
                    "tags": r.tags,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if list.is_empty() {
        println!("No capabilities in local registry.");
        println!("Use `gang registry publish` to add a capability.");
        return Ok(());
    }

    println!("{} capability(ies) in registry:\n", list.len());
    for r in &list {
        println!("  {} v{} [{}]", r.name, r.latest_version, r.language);
        println!("    {}", r.description);
    }
    Ok(())
}

/// `gang registry info <name>`
pub async fn registry_info(name: &str, format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;

    if let OutputFormat::Json = format {
        let versions = reg.get(name);
        let entries: Vec<_> = versions
            .map(|vs| {
                vs.iter()
                    .map(|e| {
                        serde_json::json!({
                            "name": e.name,
                            "version": e.version,
                            "description": e.description,
                            "author": e.author_peer_id,
                            "language": e.language.to_string(),
                            "published_at": e.published_at,
                            "component_cid": e.component_cid.as_str(),
                            "manifest_cid": e.manifest_cid.as_str(),
                            "declared_capabilities": e.declared_capabilities,
                            "tags": e.tags,
                            "min_ganglion_version": e.min_ganglion_version,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": name,
                "found": !entries.is_empty(),
                "versions": entries,
            }))?
        );
        return Ok(());
    }

    match reg.get(name) {
        Some(versions) => {
            println!("Capability: {name}\n");
            for entry in versions {
                println!("  v{}", entry.version);
                println!("    Description:   {}", entry.description);
                println!("    Author:        {}", entry.author_peer_id);
                println!("    Language:       {}", entry.language);
                println!("    Published:     {}", entry.published_at);
                println!("    Component CID: {}", entry.component_cid);
                if !entry.declared_capabilities.is_empty() {
                    println!(
                        "    Capabilities:  {}",
                        entry.declared_capabilities.join(", ")
                    );
                }
                if !entry.tags.is_empty() {
                    println!("    Tags:          {}", entry.tags.join(", "));
                }
                if let Some(min_ver) = &entry.min_ganglion_version {
                    println!("    Min Ganglion:  {min_ver}");
                }
                println!();
            }
        }
        None => {
            eprintln!("{name} not found in registry.");
        }
    }
    Ok(())
}

/// `gang relay` — run a circuit relay v2 server.
pub async fn relay(
    listen_addrs: Option<Vec<String>>,
    port: u16,
    metrics_port: u16,
    data_dir: Option<&str>,
) -> anyhow::Result<()> {
    use gang_libp2p::Libp2pConfig;

    // Resolve the identity key path. With --data-dir we persist the relay
    // identity there and plumb the path explicitly into the Libp2pConfig
    // below — no environment mutation. (GANG_KEY_PATH remains supported for
    // reads via gang-core's default_key_path, e.g. for `gang identity show`.)
    let key_path = match data_dir {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creating data dir {}", dir.display()))?;
            dir.join("identity.key")
        }
        None => gang_core::identity::default_key_path(),
    };
    let keypair = gang_core::identity::Keypair::load_or_generate(&key_path)?;
    let peer_id = keypair.peer_id();

    // Build listen addresses from explicit addrs or port shorthand
    let addrs = match listen_addrs {
        Some(addrs) if !addrs.is_empty() => addrs,
        _ => vec![
            format!("/ip4/0.0.0.0/tcp/{port}"),
            format!("/ip4/0.0.0.0/udp/{port}/quic-v1"),
        ],
    };

    let config = Libp2pConfig {
        key_path,
        listen_addrs: addrs.clone(),
        relay_server: true,
        ..Default::default()
    };

    // Create the transport adapter first so the printed client-config
    // multiaddrs can carry the LIBP2P (base58 `12D3KooW…`) peer id — the only
    // form that is dialable in a `/p2p/` multiaddr component. The gang peer id
    // (`12D3-<hex>`) identifies this relay in trust stores and policy, but a
    // multiaddr built with it will not parse.
    let adapter = gang_libp2p::Libp2pTransportAdapter::new(config).await?;
    let libp2p_peer_id = *adapter.libp2p_peer_id();

    println!("Ganglion Relay Server");
    println!("====================");
    println!();
    println!("Peer ID (gang identity): {peer_id}");
    println!("Peer ID (libp2p/dial):   {libp2p_peer_id}");
    println!("Relay mode:   server");
    println!("Metrics port: {metrics_port} (not yet active)");
    println!();
    println!("Listen addresses:");
    for addr in &addrs {
        println!("  {addr}");
    }
    println!();

    // Print the relay multiaddr that clients should use (dialable form).
    println!("Relay multiaddrs (for client config):");
    for addr in &addrs {
        println!("  {addr}/p2p/{libp2p_peer_id}");
    }
    println!();

    println!("Relay is running. Press Ctrl+C to stop.");
    println!();

    // Run the event loop until interrupted
    tokio::select! {
        result = adapter.run_event_loop() => {
            if let Err(e) = result {
                eprintln!("Event loop error: {e}");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nRelay stopped.");
        }
    }

    Ok(())
}

pub(crate) fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m}m")
    } else if secs >= 60 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s}s")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod wit_sync_tests {
    /// The vendored WIT copy in this crate must stay identical to the
    /// canonical one in gang-wasm-host. Runtime read (not include_str!) so
    /// packaged builds outside the workspace skip it gracefully.
    #[test]
    fn vendored_wit_matches_canonical() {
        let vendored = include_str!("../wit/ganglion.wit");
        let canonical_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gang-wasm-host/wit/ganglion.wit");
        if canonical_path.exists() {
            let canonical = std::fs::read_to_string(canonical_path).unwrap();
            assert_eq!(
                vendored, canonical,
                "run: cp crates/gang-wasm-host/wit/ganglion.wit crates/gang-cli/wit/"
            );
        }
    }
}
