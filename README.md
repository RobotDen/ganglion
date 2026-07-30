# Ganglion

Hostile-network reachability and sandboxed field tooling for ROS 2 robot fleets.

![status: usable](https://img.shields.io/badge/status-usable-green) [![crates.io](https://img.shields.io/crates/v/gang.svg)](https://crates.io/crates/gang) [![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

![gang demo](docs/assets/ganglion-demo.gif)
Ganglion is the connectivity and tool-execution substrate for reaching robots deployed inside customer networks you don't own and can't configure. It is not a fleet management platform, not a robot autonomy framework, and not a SaaS product.

## Architecture

Three layers:

1. **Connectivity (Layer 1)** — libp2p peer identity, secure channels, transports (TCP + QUIC), circuit relay v2, NAT traversal (DCUtR hole-punching). Robots dial out; operators reach robots via a shared relay. Inbound connectivity is never assumed.

2. **Tool Execution (Layer 2)** — WASM component runtime (Wasmtime). Signed, sandboxed, versioned tools with explicit capability declarations. No ambient authority. Memory limits, CPU budgets, and wall-clock deadlines enforced per-component.

3. **Native Brokers (Layer 3)** — Privileged host processes that mediate WASM capability access to ROS 2 topics/services/parameters, filesystem, system logs, diagnostics, subprocess execution, network probing, and metrics. Pattern-based access gating with default-deny policy.

## Quick start

```bash
# Install the CLI from crates.io (Rust 1.88+)
cargo install gang

# ...or build from source (see CONTRIBUTING.md):
#   git clone https://github.com/RobotDen/ganglion.git
#   cd ganglion && cargo install --path crates/gang-cli

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
│   ├── gang-capability-canary-probe/       # Fleet-scale canary health probe
│   └── gang-capability-rosbag-slice/       # Time-bounded rosbag2 slicing
├── examples/                               # Multi-language reference implementations
│   ├── python/                             # Python log-normalize (componentize-py)
│   ├── cpp/                                # C++ topic-echo (wasi-sdk + wit-bindgen)
│   └── go/                                 # Go canary-probe (TinyGo)
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
├── deploy/
│   └── relay/                              # Bootstrap circuit-relay v2 Docker deployment
├── scripts/                               # Developer scripts (e.g. setup-hooks.sh)
├── .github/workflows/
│   ├── ci.yml                              # CI: check, fmt, clippy, test, doc
│   └── release.yml                         # Release: validate + GitHub Release on tags
├── .githooks/
│   └── pre-commit                          # Pre-commit: fmt, clippy -Dwarnings, test
├── CHANGELOG.md                            # Release history
├── CLAUDE.md                               # Agent/contributor working notes
├── CONTRIBUTING.md                         # Contribution guidelines
├── LICENSE                                 # Apache-2.0
└── NOTICE                                  # Apache-2.0 attribution notice
```

## CLI commands

```
gang identity show                    Show your PeerId and public key
gang identity generate [--force]      Generate a new Ed25519 keypair
gang sign <wasm> [--capabilities C]   Sign a WASM component, produce .manifest.cbor
                 [--key K] [--name N] [--component-version V]
gang agent [--data-dir] [-r relay]    Run a robot agent (local or remote mode)
gang deploy <robot> <wasm>            Deploy a signed capability to a robot
gang run <robot> <cap> [args...]      Invoke an installed capability
gang caps <robot>                     List capabilities installed on a robot
gang logs <robot> [--follow]          Stream robot logs [WIP]
gang demo                             Self-contained end-to-end demo
gang diagnose [robot]                 Detect network archetype, recommend transport config
gang transport-stats <robot>          Show per-transport connection statistics [WIP: simulated]
gang test-archetype <archetype>       Launch a Docker network scenario
gang fetch <cid> [-o path]            Retrieve an artifact by CID
gang push <path> [--content-type T]   Publish a file to the content store
gang artifacts                        List locally-stored artifacts
gang capability scaffold <name>       Generate a capability project skeleton
gang registry search <query>          Search the capability registry
gang registry install <name>          Install a capability from the registry
gang registry publish <wasm>          Publish a capability (signed manifest required)
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
- Default-deny policy engine with glob pattern matching; policy and trust store fail closed (malformed files abort startup)
- Tamper-evident audit logging: Blake3 hash chain with `verify_chain()` integrity check, `0600` permissions, size-based rotation
- Replay protection (nonce + timestamp) on control requests
- Canonicalized (TOCTOU-closed) symlink jail on filesystem access
- Absolute-path command allowlist and environment scrubbing on subprocess execution
- Network-probe SSRF guards (loopback / link-local / cloud-metadata blocked) with host allowlist
- Fuel metering, manifest-derived memory limits, and epoch-based wall-clock deadlines for WASM execution (component bytes re-hashed before execution)

See [docs/SECURITY.md](docs/SECURITY.md) for the full threat model.

## Building

```bash
# Prerequisites: Rust 1.88+, cargo
# System deps (Debian/Ubuntu): sudo apt-get install pkg-config libssl-dev
cargo build --release

# Run all tests (337 tests across 13 crates; 1 additional live-network
# archetype test is ignored by default — run it with `cargo test -- --ignored`)
cargo test

# Run with warnings as errors (matches CI)
RUSTFLAGS="-Dwarnings" cargo clippy --all-targets

# Build documentation
cargo doc --no-deps --open

# Set up git hooks
./scripts/setup-hooks.sh
```

## Using Ganglion in production?

Ganglion is Apache-2.0 — run it, fork it, no strings. If it saves you a truck roll, [sponsoring the den](https://github.com/sponsors/karma0) is a welcome thanks.

When one engineer's setup becomes a fleet with auditors, SLAs, and many sites, that's the line where the open core ends and [FleetLink](https://tafylabs.io) begins: the enterprise layer built on this substrate (multi-tenant gateway, hosted/HA relay, SSO and RBAC, compliance reporting, fleet-scale OTA) plus fixed-scope architecture reviews and implementations by the people who wrote Ganglion. Start at [tafylabs.io](https://tafylabs.io) or email bobby@tafy.ai.

## License

Apache-2.0
