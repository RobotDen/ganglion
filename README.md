# Ganglion

Hostile-network reachability and sandboxed field tooling for ROS 2 robot fleets.

Ganglion is the connectivity and tool-execution substrate for reaching robots deployed inside customer networks you don't own and can't configure. It is not a fleet management platform, not a robot autonomy framework, and not a SaaS product.

## Architecture

Three layers:

1. **Connectivity (Layer 1)** — libp2p peer identity, secure channels, transports (TCP + QUIC), circuit relay v2, NAT traversal (DCUtR hole-punching). Robots dial out; operators reach robots via a shared relay. Inbound connectivity is never assumed.

2. **Tool Execution (Layer 2)** — WASM component runtime (Wasmtime). Signed, sandboxed, versioned tools with explicit capability declarations. No ambient authority. Memory limits, CPU budgets, and wall-clock deadlines enforced per-component.

3. **Native Brokers (Layer 3)** — Privileged host processes that mediate WASM capability access to ROS 2 topics/services/parameters, filesystem, system logs, diagnostics, subprocess execution, network probing, and metrics. Pattern-based access gating with default-deny policy.

## Quick start

```bash
# Build from source (Rust 1.85+)
cargo install --path crates/gang-cli

# Run the self-contained demo — no Docker, no ROS 2, no external dependencies
gang demo

# Diagnose local network archetype
gang diagnose

# Or run a network archetype test scenario (requires Docker)
gang test-archetype open-warehouse
```

See [docs/QUICKSTART.md](docs/QUICKSTART.md) for a detailed walkthrough.

## Workspace structure

```
ganglion/
├── crates/
│   ├── gang-core/                          # Core types: identity, messages, policy, audit, manifests, registry
│   ├── gang-libp2p/                        # libp2p transport adapter (TCP, QUIC, relay, DCUtR)
│   ├── gang-wasm-host/                     # Wasmtime component runtime with WIT interfaces
│   ├── gang-ros/                           # ROS 2 brokers, robot agent, archetype detection
│   ├── gang-cli/                           # `gang` CLI binary
│   ├── gang-capability-diagnostics/        # Basic system diagnostics
│   ├── gang-capability-param-inspect/      # ROS 2 parameter snapshot and diff
│   ├── gang-capability-diagnostic-bundle/  # Comprehensive diagnostic bundle with health checks
│   ├── gang-capability-network-archetype/  # Network archetype detection with connectivity scoring
│   ├── gang-capability-log-normalize/      # Log format normalization (journald, ROS 2, syslog)
│   ├── gang-capability-topic-echo/         # ROS 2 topic capture with decimation
│   └── gang-capability-canary-probe/       # Fleet-scale canary health probe
├── test-harness/                           # Docker-compose network archetype scenarios
│   ├── open-warehouse/                     # Flat L2, direct connectivity
│   ├── nat-office/                         # Consumer NAT, relay + DCUtR
│   ├── enterprise-dmz/                     # VLAN isolation, TCP 443 only
│   └── mobile-cgnat/                       # Symmetric NAT, CGNAT, jitter + loss
├── docs/
│   ├── ARCHITECTURE.md                     # Full architectural reference
│   ├── SECURITY.md                         # Threat model and security design
│   ├── CLI_REFERENCE.md                    # Complete CLI documentation
│   ├── NETWORK_ARCHETYPES.md               # Network archetype deep dive
│   ├── CAPABILITY_AUTHOR_GUIDE.md          # Writing capabilities in Rust, C++, Python, Go
│   ├── QUICKSTART.md                       # 5-minute getting-started guide
│   ├── VALIDATION.md                       # Test harness results and measurements
│   ├── DesignSpec.md                       # Original design specification
│   ├── IMPLEMENTATION.md                   # Implementation plan and progress
│   └── adr/                                # Architecture Decision Records (ADR-001 to ADR-020)
├── .github/workflows/
│   ├── ci.yml                              # CI: check, fmt, clippy, test, doc
│   └── release.yml                         # Release: validate + GitHub Release on tags
├── .githooks/
│   └── pre-commit                          # Pre-commit: fmt, clippy -Dwarnings, test
└── CONTRIBUTING.md                         # Contribution guidelines
```

## CLI commands

```
gang identity show                    Show your PeerId and public key
gang identity generate [--force]      Generate a new Ed25519 keypair
gang sign <wasm> [--key K] [--name N] Sign a WASM component, produce .manifest.cbor
gang agent [--data-dir] [-r relay]    Run a robot agent (local or remote mode)
gang deploy <robot> <wasm>            Deploy a signed capability to a robot
gang run <robot> <cap> [args...]      Invoke an installed capability
gang caps <robot>                     List capabilities installed on a robot
gang logs <robot> [--follow]          Stream robot logs
gang demo                             Self-contained end-to-end demo
gang diagnose [robot]                 Detect network archetype, recommend transport config
gang transport-stats <robot>          Show per-transport connection statistics
gang test-archetype <archetype>       Launch a Docker network scenario
gang fetch <cid> [-o path]            Retrieve an artifact by CID
gang push <path> [--content-type T]   Publish a file to the content store
gang artifacts                        List locally-stored artifacts
gang capability scaffold <name>       Generate a capability project skeleton
gang registry search <query>          Search the capability registry
gang registry install <name>          Install a capability from the registry
gang registry publish <wasm>          Publish a capability to the registry
gang registry list                    List all registry capabilities
gang registry info <name>             Show capability details
gang peer add <name> <peer-id>        Register a known peer
gang peer remove <name>               Remove a registered peer
gang peer list                        List all registered peers
gang peer show <name>                 Show details for a specific peer
gang peer rename <old> <new>          Rename a registered peer
gang config show                      Show current configuration
gang config set <key> <value>         Set a configuration value
gang config init [--force]            Initialize default config file
gang completions <shell>              Generate shell completions (bash/zsh/fish)
gang relay [--port P]                 Run a circuit relay v2 server
gang list                             List reachable robots in the fleet [WIP]
gang connect <robot>                  Establish a session via relay [WIP]
```

See [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md) for full details, flags, and examples.

## Capability groups

Ganglion defines eight WIT capability interfaces that WASM components can declare:

| Group | WIT interface | Description |
|-------|--------------|-------------|
| ROS Interface | `ganglion:ros/interface` | Topic subscribe, service call, parameter get/set |
| Log Stream | `ganglion:logs/stream` | Journald, syslog, ROS log file access |
| FS Bounded | `ganglion:fs/bounded` | Symlink-jailed filesystem access |
| Diagnostics | `ganglion:diagnostics/collect` | System info, process lists, network state |
| Artifacts | `ganglion:artifacts/publish` | Content-addressed artifact store |
| Process Spawn | `ganglion:process/spawn` | Bounded subprocess invocation with command allowlist |
| Network Probe | `ganglion:network/probe` | Ping, DNS, TCP port check, traceroute |
| Metrics Emit | `ganglion:metrics/emit` | Structured metric emission |

All interfaces are part of the `ganglion:capability@0.5.0` WIT package. Individual interfaces are not independently versioned.

## Network archetypes

Ganglion is designed around five real-world network environments:

| Archetype | Characteristics | Ganglion strategy |
|-----------|----------------|-------------------|
| Open warehouse | Flat L2, no NAT | Direct TCP/QUIC |
| NAT'd office | Consumer NAT, no inbound | Relay + DCUtR hole-punch |
| Enterprise DMZ | VLAN isolation, port restrictions | Relay on TCP 443 |
| Regulated facility | Air-gapped, physical sneakernet | Offline signed bundles |
| Mobile/CGNAT | Symmetric NAT, IP rotation | Relay-only, reconnect logic |

See [docs/NETWORK_ARCHETYPES.md](docs/NETWORK_ARCHETYPES.md) for a deep dive.

## Design principles

1. **Protocol-agnostic core, opinionated defaults.** libp2p is the default transport, not the only valid one.
2. **Capability-bounded remote execution.** All remote operations use signed, sandboxed WASM components with explicit capability declarations.
3. **Outbound-initiated by default.** Robots dial out. Operators reach robots via a shared relay.
4. **Operability before novelty.** Every feature must be debuggable from a single-operator laptop.
5. **Honest OSS boundary.** The reference demonstrates correctness; commercial products provide durability and governance.

## Security model

- Ed25519 keypair identity with PeerId derivation (Blake3 hash of public key)
- Noise protocol encrypted channels (libp2p)
- Signed WASM component manifests with trust store verification
- Default-deny policy engine with glob pattern matching
- Append-only audit logging with size-based rotation
- Symlink jail enforcement on filesystem access
- Command allowlist on subprocess execution
- Fuel metering and epoch-based wall-clock deadlines for WASM execution

See [docs/SECURITY.md](docs/SECURITY.md) for the full threat model.

## Building

```bash
# Prerequisites: Rust 1.85+, cargo
cargo build --release

# Run all tests (188 tests across 9 crates)
cargo test

# Run with warnings as errors (matches CI)
RUSTFLAGS="-Dwarnings" cargo clippy --all-targets

# Build documentation
cargo doc --no-deps --open

# Set up git hooks
./scripts/setup-hooks.sh
```

## License

Apache-2.0
