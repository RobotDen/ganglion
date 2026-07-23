use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::OutputFormat;

// --- Operator config ---

/// Operator configuration loaded from `~/.gang/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperatorConfig {
    /// Default relay multiaddr when --relay is not specified and the peer
    /// registry entry has no relay_addrs.
    pub default_relay: Option<String>,

    /// Identity verification policy: "strict" (default), "tofu", or "none".
    #[serde(default = "default_host_key_policy")]
    pub host_key_policy: String,
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

    let available = [
        "identity show",
        "identity generate",
        "sign",
        "agent",
        "deploy",
        "run",
        "caps",
        "demo",
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
    ];

    // WIP: require relay connectivity or produce only simulated output today.
    let wip = ["logs", "list", "connect", "transport-stats (simulated data)"];

    match format {
        OutputFormat::Json => {
            let info = serde_json::json!({
                "version": version,
                "identity": identity_status,
                "key_path": key_path.display().to_string(),
                "registry_capabilities": registry_count,
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
            println!();
            println!("WIP commands (require relay connectivity):");
            for cmd in &wip {
                println!("  gang {cmd}  [WIP]");
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

// --- Target resolution ---

/// Resolved target for a robot command.
#[allow(dead_code)] // relay_addr used when remote dispatch is wired (ADR-020 Phase 32)
pub struct ResolvedTarget {
    /// The full peer ID (if remote).
    pub peer_id: Option<gang_core::identity::PeerId>,
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

    // 1. Explicit --peer flag
    if let Some(peer_str) = explicit_peer {
        return Ok(ResolvedTarget {
            peer_id: Some(PeerId::new(peer_str)),
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
        return Ok(ResolvedTarget {
            peer_id: Some(PeerId::new(robot)),
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
pub async fn peer_add(
    name: &str,
    peer_id_str: &str,
    relay: Option<&str>,
    role_str: &str,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::identity::{PeerEntry, PeerId, PeerRegistry, Role, default_registry_path};

    let peer_id = PeerId::new(peer_id_str);
    if !peer_id.as_str().starts_with("12D3-") || peer_id.as_str().len() != 37 {
        anyhow::bail!(
            "Invalid peer ID: '{}'. Expected format: 12D3-<32 hex chars>",
            peer_id_str
        );
    }
    // Validate the 32-char hex payload after the "12D3-" prefix.
    let hex_payload = &peer_id.as_str()[5..];
    if hex::decode(hex_payload).is_err() {
        anyhow::bail!(
            "Invalid peer ID: '{}'. The 32 characters after '12D3-' must be hex.",
            peer_id_str
        );
    }

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
                    "role": role_str,
                })
            );
        }
        OutputFormat::Text => {
            println!("Registered peer '{name}':");
            println!("  Peer ID: {peer_id}");
            println!("  Role:    {role_str}");
            if let Some(r) = relay {
                println!("  Relay:   {r}");
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
                "{:<16} {:<16} {:<14} {}",
                "NAME", "PEER ID", "ROLE", "RELAY"
            );
            println!("{header}");
            for (name, entry) in &peers {
                let abbrev = if entry.peer_id.as_str().len() > 16 {
                    &entry.peer_id.as_str()[..16]
                } else {
                    entry.peer_id.as_str()
                };
                let relay = entry
                    .relay_addrs
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("(none)");
                println!("{:<16} {:<16} {:<14} {}", name, abbrev, entry.role, relay);
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
                    "role": format!("{}", entry.role),
                    "relay_addrs": entry.relay_addrs,
                })
            );
        }
        OutputFormat::Text => {
            println!("Peer '{name}':");
            println!("  Peer ID:  {}", entry.peer_id);
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
                "note: host_key_policy is stored but not yet enforced; it takes effect \
                 once remote connections land."
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
            if key == "host_key_policy" {
                println!(
                    "note: host_key_policy is stored but not yet enforced; it takes effect \
                     once remote connections land."
                );
            }
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
#[allow(dead_code)] // Called by remote dispatch when connections are wired (ADR-020 Phase 32)
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
#[allow(dead_code)]
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

/// Parse a capability group short name (or fully-qualified name) into a
/// `CapabilityGroup` with permissive defaults for any pattern/allowlist fields.
fn parse_capability_group(
    spec: &str,
) -> anyhow::Result<gang_core::capability::CapabilityGroup> {
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
        "artifacts" | "ganglion:artifacts/publish" => {
            CapabilityGroup::ArtifactsPublish { version }
        }
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
        println!("Register on operator machine:");
        println!("  gang peer add my-robot {peer_id} --relay {relay_addr}");
        println!();
        println!("Starting transport...");

        // Create libp2p transport with agent identity
        let transport_config = gang_libp2p::Libp2pConfig {
            key_path: data_dir.join("identity.key"),
            relay_addrs: vec![relay_addr.to_string()],
            ..Default::default()
        };

        let transport = gang_libp2p::Libp2pTransportAdapter::new(transport_config).await?;

        // Register the control protocol handler
        agent.serve(&transport).await?;

        // Dial the relay to establish a circuit reservation
        transport
            .dial_multiaddr(relay_addr)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to relay {relay_addr}: {e}"))?;

        println!("Connected to relay. Waiting for operator connections...");
        println!("Press Ctrl+C to stop.");

        // Run the event loop alongside Ctrl+C
        tokio::select! {
            result = transport.run_event_loop() => {
                if let Err(e) = result {
                    eprintln!("Transport event loop error: {e}");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nAgent stopped.");
            }
        }
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
pub async fn deploy(
    robot: &str,
    wasm_path: &str,
    manifest_path: Option<&str>,
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
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
        // Remote dispatch — will be implemented when agent serve loop is ready (Phase 32).
        let peer_id = target.peer_id.as_ref().unwrap();
        let display_name = target.name.as_deref().unwrap_or(peer_id.as_str());
        anyhow::bail!(
            "Remote deploy to '{display_name}' ({peer_id}) is not yet implemented.\n\
             The transport infrastructure is ready but the agent serve loop (ADR-020 Phase 32) \n\
             must be completed first. Use local mode for now:\n\
             \n\
             gang deploy {robot} {}",
            wasm_path.display()
        );
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
pub async fn run(
    robot: &str,
    cap_name: &str,
    args: &[String],
    explicit_peer: Option<&str>,
    explicit_relay: Option<&str>,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;

    let target = resolve_target(robot, explicit_peer, explicit_relay)?;

    if !target.is_local {
        let peer_id = target.peer_id.as_ref().unwrap();
        let display_name = target.name.as_deref().unwrap_or(peer_id.as_str());
        anyhow::bail!(
            "Remote run on '{display_name}' ({peer_id}) is not yet implemented.\n\
             The transport infrastructure is ready but the agent serve loop (ADR-020 Phase 32) \n\
             must be completed first. Use local mode for now:\n\
             \n\
             gang run {robot} {cap_name}"
        );
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
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_ros::agent::{AgentConfig, RobotAgent};

    let target = resolve_target(robot, explicit_peer, explicit_relay)?;

    if !target.is_local {
        let peer_id = target.peer_id.as_ref().unwrap();
        let display_name = target.name.as_deref().unwrap_or(peer_id.as_str());
        anyhow::bail!(
            "Remote caps on '{display_name}' ({peer_id}) is not yet implemented.\n\
             The agent serve loop (ADR-020 Phase 32) must be completed first."
        );
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

    // Wait for stabilization
    println!("Waiting for services to stabilize...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

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
        if let Some(mem) = sys.get("memory_total_bytes").and_then(|v| v.as_u64()) {
            if mem > 0 {
                println!("  Memory:    {} GB", mem / (1024 * 1024 * 1024));
            }
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

    if let Some(net) = val.get("network") {
        if let Some(interfaces) = net.get("interfaces").and_then(|v| v.as_array()) {
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

/// `gang transport-stats` — show per-transport statistics for a peer.
pub async fn transport_stats(robot: &str, format: &crate::OutputFormat) -> anyhow::Result<()> {
    // For now, show simulated stats since we don't have a live connection.
    // In full implementation, this queries the transport adapter for the
    // connected peer's stats.

    let example_stats = gang_core::transport::TransportStats {
        transport: "quic".into(),
        via_relay: false,
        connect_time_ms: 145,
        messages_sent: 42,
        messages_received: 38,
        bytes_sent: 12_480,
        bytes_received: 156_320,
        last_rtt_ms: Some(23),
        dcutr_attempted: true,
        dcutr_succeeded: true,
        uptime_secs: 3600,
        reconnections: 0,
    };

    match format {
        crate::OutputFormat::Json => {
            // Mark the payload as simulated so machine consumers cannot mistake
            // it for live data (CLI-03).
            let mut json = serde_json::to_value(&example_stats)?;
            if let Some(obj) = json.as_object_mut() {
                obj.insert("simulated".into(), serde_json::json!(true));
                obj.insert("peer".into(), serde_json::json!(robot));
            }
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        crate::OutputFormat::Text => {
            println!("Transport statistics for: {robot}  [WIP]");
            println!("(No live connection — showing SIMULATED example output.)");
            println!();
            println!("  Transport:       {}", example_stats.transport);
            println!("  Via relay:       {}", example_stats.via_relay);
            println!("  Connect time:    {}ms", example_stats.connect_time_ms);
            println!(
                "  Messages:        {} sent, {} received",
                example_stats.messages_sent, example_stats.messages_received
            );
            println!(
                "  Bytes:           {} sent, {} received",
                format_bytes(example_stats.bytes_sent),
                format_bytes(example_stats.bytes_received)
            );
            if let Some(rtt) = example_stats.last_rtt_ms {
                println!("  Last RTT:        {rtt}ms");
            }
            println!(
                "  DCUtR:           attempted={}, succeeded={}",
                example_stats.dcutr_attempted, example_stats.dcutr_succeeded
            );
            println!(
                "  Uptime:          {}",
                format_duration(example_stats.uptime_secs)
            );
            println!("  Reconnections:   {}", example_stats.reconnections);
        }
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
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

    let cid = Cid::parse(cid_str);
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
    // from the in-repo canonical copy so scaffolds are usable out of the box.
    const GANGLION_WIT: &str = include_str!("../../gang-wasm-host/wit/ganglion.wit");
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
    let (manifest_cid, manifest) = if manifest_path.exists() {
        let manifest_bytes = std::fs::read(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let cid = gang_core::artifacts::Cid::from_bytes(&manifest_bytes);
        let decoded = SignedManifest::from_cbor(&manifest_bytes)
            .and_then(|s| s.verify_and_decode())
            .with_context(|| format!("decoding manifest {}", manifest_path.display()))?;
        (cid, Some(decoded))
    } else {
        // No manifest found; compute CID from the component bytes as fallback
        (gang_core::artifacts::Cid::from_bytes(&data), None)
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

    let min_ganglion_version = manifest.as_ref().and_then(|m| m.min_ganglion_version.clone());

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

    let mut reg = gang_core::registry::Registry::open(&registry_dir())?;
    reg.publish(entry)?;

    println!("Published {name} v{version} to local registry.");
    if manifest.is_none() {
        println!(
            "  (no signed manifest found next to the component — used filename/defaults; \
             sign it first for accurate metadata)"
        );
    }
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
pub async fn registry_info(name: &str, _format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;

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
) -> anyhow::Result<()> {
    use gang_libp2p::Libp2pConfig;

    // Load or generate identity
    let key_path = gang_core::identity::default_key_path();
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

    println!("Ganglion Relay Server");
    println!("====================");
    println!();
    println!("Peer ID:      {peer_id}");
    println!("Relay mode:   server");
    println!("Metrics port: {metrics_port} (not yet active)");
    println!();
    println!("Listen addresses:");
    for addr in &addrs {
        println!("  {addr}");
    }
    println!();

    // Print the relay multiaddr that clients should use
    println!("Relay multiaddrs (for client config):");
    for addr in &addrs {
        println!("  {addr}/p2p/{peer_id}");
    }
    println!();

    // Create the transport adapter and run
    let adapter = gang_libp2p::Libp2pTransportAdapter::new(config).await?;

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

fn format_duration(secs: u64) -> String {
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
