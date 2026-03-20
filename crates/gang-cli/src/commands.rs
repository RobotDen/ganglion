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
    // Search order: ./test-harness, ../test-harness, ../../test-harness
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

    // Build
    println!("Building container images...");
    let build_status = std::process::Command::new("docker")
        .args([
            "compose",
            "-p", &project_name,
            "-f", &compose_file.to_string_lossy(),
            "build",
        ])
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
            "-p", &project_name,
            "-f", &compose_file.to_string_lossy(),
            "up", "-d",
        ])
        .status()?;

    if !up_status.success() {
        anyhow::bail!("Failed to start scenario. Check output above.");
    }

    // Wait for stabilization
    println!("Waiting for services to stabilize...");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Show service status
    println!();
    let _ = std::process::Command::new("docker")
        .args([
            "compose",
            "-p", &project_name,
            "-f", &compose_file.to_string_lossy(),
            "ps",
        ])
        .status();

    // Show logs
    println!();
    println!("=== Service logs ===");
    let _ = std::process::Command::new("docker")
        .args([
            "compose",
            "-p", &project_name,
            "-f", &compose_file.to_string_lossy(),
            "logs", "--tail", "20",
        ])
        .status();

    println!();
    println!("============================================");
    println!("  Scenario {archetype} is running");
    println!("============================================");
    println!();
    println!("Inspect manually:");
    println!("  docker compose -p {project_name} -f {} exec robot bash",
             compose_file.display());
    println!("  docker compose -p {project_name} -f {} logs -f",
             compose_file.display());
    println!();
    println!("Tear down:");
    println!("  docker compose -p {project_name} -f {} down -v",
             compose_file.display());

    Ok(())
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

/// `gang diagnose` — detect network archetype and recommend transport config.
pub async fn diagnose(
    robot: Option<&str>,
    format: &crate::OutputFormat,
) -> anyhow::Result<()> {
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
            println!("  Detected:    {} ({:.0}% confidence)",
                     result.archetype, result.confidence * 100.0);
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
pub async fn transport_stats(
    robot: &str,
    format: &crate::OutputFormat,
) -> anyhow::Result<()> {
    // For now, show simulated stats since we don't have a live connection.
    // In full implementation, this queries the transport adapter for the
    // connected peer's stats.

    println!("Transport statistics for: {robot}");
    println!("(Requires active connection — showing example output)");
    println!();

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
            let json = serde_json::to_string_pretty(&example_stats)?;
            println!("{json}");
        }
        crate::OutputFormat::Text => {
            println!("  Transport:       {}", example_stats.transport);
            println!("  Via relay:       {}", example_stats.via_relay);
            println!("  Connect time:    {}ms", example_stats.connect_time_ms);
            println!("  Messages:        {} sent, {} received",
                     example_stats.messages_sent, example_stats.messages_received);
            println!("  Bytes:           {} sent, {} received",
                     format_bytes(example_stats.bytes_sent),
                     format_bytes(example_stats.bytes_received));
            if let Some(rtt) = example_stats.last_rtt_ms {
                println!("  Last RTT:        {rtt}ms");
            }
            println!("  DCUtR:           attempted={}, succeeded={}",
                     example_stats.dcutr_attempted, example_stats.dcutr_succeeded);
            println!("  Uptime:          {}",
                     format_duration(example_stats.uptime_secs));
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
    _format: &crate::OutputFormat,
) -> anyhow::Result<()> {
    use gang_core::artifacts::{ArtifactStore, ArtifactStoreConfig, Cid};

    let store_dir = artifact_store_dir();
    let mut store = ArtifactStore::open(ArtifactStoreConfig {
        store_dir,
        ..Default::default()
    })?;

    let cid = Cid::from_str(cid_str);
    if !store.contains(&cid) {
        anyhow::bail!(
            "Artifact {cid_str} not found in local store.\n\
             Remote fetch from peers is not yet implemented."
        );
    }

    let data = store.retrieve(&cid)?;
    let meta = store.meta(&cid);

    match output {
        Some(path) => {
            std::fs::write(path, &data)?;
            println!("Wrote {} bytes to {path}", data.len());
        }
        None => {
            let filename = meta
                .and_then(|m| m.filename.as_deref())
                .unwrap_or("artifact.bin");
            std::fs::write(filename, &data)?;
            println!("Wrote {} bytes to {filename}", data.len());
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

    let data = std::fs::read(path)?;
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str());

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
                println!("Stored artifacts ({}, {}):",
                         artifacts.len(),
                         format_bytes(store.total_bytes()));
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

/// Default registry directory.
fn registry_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("gang")
        .join("registry")
}

/// `gang registry search <query>`
pub async fn registry_search(query: &str, _format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;
    let results = reg.search(query);

    if results.is_empty() {
        println!("No capabilities found matching \"{query}\".");
        return Ok(());
    }

    println!("Found {} result(s) for \"{}\":\n", results.len(), query);
    for r in &results {
        println!("  {} v{}", r.name, r.latest_version);
        println!("    {}", r.description);
        println!("    Language: {}  Author: {}...{}", r.language, &r.author[..8.min(r.author.len())], &r.author[r.author.len().saturating_sub(4)..]);
        if !r.tags.is_empty() {
            println!("    Tags: {}", r.tags.join(", "));
        }
        println!();
    }
    Ok(())
}

/// `gang registry install <name>`
pub async fn registry_install(name: &str, version: Option<&str>, _format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;

    let entry = if let Some(ver) = version {
        reg.get(name).and_then(|versions| versions.iter().find(|e| e.version == ver))
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
            println!("Use `gang fetch {}` to retrieve the component.", entry.component_cid);
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
    _format: &OutputFormat,
) -> anyhow::Result<()> {
    let path = Path::new(wasm_path);
    if !path.exists() {
        anyhow::bail!("file not found: {wasm_path}");
    }

    // Read the component and compute CID
    let data = std::fs::read(path)?;
    let component_cid = gang_core::artifacts::Cid::from_bytes(&data);

    // Derive name from filename
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

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
        version: "0.1.0".into(),
        description: description.unwrap_or("A Ganglion capability").into(),
        author_peer_id: author,
        language: gang_core::registry::CapabilityLanguage::Rust,
        component_cid: component_cid.clone(),
        manifest_cid: gang_core::artifacts::Cid::from_bytes(b"manifest-placeholder"),
        declared_capabilities: vec![],
        published_at: chrono::Utc::now().to_rfc3339(),
        tags: tags.map(|t| t.to_vec()).unwrap_or_default(),
        min_ganglion_version: Some("0.4.0".into()),
    };

    let mut reg = gang_core::registry::Registry::open(&registry_dir())?;
    reg.publish(entry)?;

    println!("Published {} to local registry.", name);
    println!("  Component CID: {}", component_cid);
    println!("  Registry path: {}", registry_dir().display());
    Ok(())
}

/// `gang registry list`
pub async fn registry_list(_format: &OutputFormat) -> anyhow::Result<()> {
    let reg = gang_core::registry::Registry::open(&registry_dir())?;
    let list = reg.list();

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
                    println!("    Capabilities:  {}", entry.declared_capabilities.join(", "));
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
