use std::path::{Path, PathBuf};

use crate::OutputFormat;

/// `gang identity show`
pub async fn identity_show() -> anyhow::Result<()> {
    let key_path = gang_core::identity::default_key_path();
    if !key_path.exists() {
        eprintln!("No identity found. Run `gang identity generate` first.");
        eprintln!("Expected key at: {}", key_path.display());
        std::process::exit(1);
    }

    let keypair = gang_core::identity::Keypair::load(&key_path)?;
    println!("Peer ID:    {}", keypair.peer_id());
    println!("Public key: {}", hex::encode(keypair.public_key().as_bytes()));
    println!("Key file:   {}", key_path.display());
    Ok(())
}

/// `gang identity generate`
pub async fn identity_generate(force: bool) -> anyhow::Result<()> {
    let key_path = gang_core::identity::default_key_path();
    if key_path.exists() && !force {
        eprintln!("Identity already exists at {}.", key_path.display());
        eprintln!("Use --force to overwrite.");
        std::process::exit(1);
    }

    let keypair = gang_core::identity::Keypair::generate();
    keypair.save(&key_path)?;
    println!("Generated new identity:");
    println!("  Peer ID:  {}", keypair.peer_id());
    println!("  Key file: {}", key_path.display());
    Ok(())
}

/// `gang sign`
pub async fn sign(
    wasm_path: &str,
    key_path: Option<&str>,
    name: Option<&str>,
    version: &str,
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

    let name = name
        .map(String::from)
        .unwrap_or_else(|| {
            wasm_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    let manifest = ComponentManifest {
        name: name.clone(),
        version: version.into(),
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
        component_hash: component_hash.clone(),
        limits: ResourceLimits::default(),
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
    Ok(())
}

/// `gang agent` — run the robot agent locally.
pub async fn agent(_config: Option<&str>, data_dir: &str) -> anyhow::Result<()> {
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;

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

    let agent = RobotAgent::new(config)?;
    println!("Robot agent started:");
    println!("  Peer ID:  {}", agent.peer_id());
    println!("  Data dir: {}", data_dir.display());
    println!("  Policy:   permissive (dev mode)");
    println!();
    println!("Press Ctrl+C to stop.");

    // Keep running until interrupted
    tokio::signal::ctrl_c().await?;
    println!("\nAgent stopped.");
    Ok(())
}

/// `gang deploy` — deploy a capability to a local agent.
pub async fn deploy(
    robot: &str,
    wasm_path: &str,
    manifest_path: Option<&str>,
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

    // For v0.1, deploy to a local agent instance
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
    let operator_kp = gang_core::identity::Keypair::load_or_generate(
        &gang_core::identity::default_key_path(),
    )?;

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
    format: &OutputFormat,
) -> anyhow::Result<()> {
    use gang_ros::agent::{AgentConfig, RobotAgent};
    use gang_ros::filesystem::FsRule;

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
    let operator_kp = gang_core::identity::Keypair::load_or_generate(
        &gang_core::identity::default_key_path(),
    )?;

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
pub async fn caps(robot: &str, format: &OutputFormat) -> anyhow::Result<()> {
    use gang_ros::agent::{AgentConfig, RobotAgent};
    

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
                        cap.name,
                        cap.version,
                        cap.author_peer_id
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

    println!("=== Ganglion v0.1 Demo ===");
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
        println!(
            "  {} v{} ({})",
            cap.name, cap.version, cap.author_peer_id
        );
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
    let audit_log = gang_core::audit::AuditLog::new(
        data_dir.join("audit.log"),
        50 * 1024 * 1024,
    );
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

    // Cleanup
    std::fs::remove_dir_all(&data_dir)?;

    Ok(())
}

/// `gang test-archetype`
pub async fn test_archetype(archetype: &str) -> anyhow::Result<()> {
    let valid = ["open-warehouse", "nat-office", "enterprise-dmz", "mobile-cgnat"];
    if !valid.contains(&archetype) {
        anyhow::bail!(
            "Unknown archetype: {archetype}\nValid archetypes: {}",
            valid.join(", ")
        );
    }

    // Check Docker
    let docker_check = std::process::Command::new("docker")
        .args(["info"])
        .output();

    match docker_check {
        Ok(out) if out.status.success() => {}
        _ => {
            anyhow::bail!(
                "Docker is required for test-archetype but is not available.\n\
                 Install Docker and try again."
            );
        }
    }

    println!("Test archetype: {archetype}");
    println!("Docker-compose scenarios are not yet implemented.");
    println!("Use `gang demo` for a self-contained local demo.");
    println!();
    println!("The test harness will simulate:");
    match archetype {
        "open-warehouse" => {
            println!("  - Flat L2, no NAT, permissive DHCP");
            println!("  - Direct TCP/QUIC connection between operator and robot");
            println!("  - No relay needed");
        }
        "nat-office" => {
            println!("  - Single consumer NAT, no inbound ports");
            println!("  - Robot dials out to relay");
            println!("  - Operator connects via relay, DCUtR upgrade attempted");
        }
        "enterprise-dmz" => {
            println!("  - VLAN isolation, restricted outbound ports");
            println!("  - TLS inspection proxy");
            println!("  - Robot connects outbound on 443 only");
        }
        "mobile-cgnat" => {
            println!("  - Symmetric NAT, CGNAT, IP rotation");
            println!("  - Relay-only connectivity (DCUtR fails on symmetric NAT)");
            println!("  - Intermittent connectivity simulation");
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Pretty-print diagnostics output for human consumption.
fn print_diagnostics(val: &serde_json::Value) {
    if let Some(sys) = val.get("system_info") {
        println!("System Information:");
        if let Some(h) = sys.get("hostname").and_then(|v| v.as_str()) {
            println!("  Hostname:  {h}");
        }
        if let Some(os) = sys.get("os").and_then(|v| v.as_str()) {
            let ver = sys
                .get("os_version")
                .and_then(|v| v.as_str())
                .unwrap_or("");
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
            let cpu_a = a
                .get("cpu_percent")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let cpu_b = b
                .get("cpu_percent")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            cpu_b.partial_cmp(&cpu_a).unwrap_or(std::cmp::Ordering::Equal)
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
