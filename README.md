# Ganglion

Hostile-network reachability and sandboxed field tooling for ROS 2 robot fleets.

Ganglion is the connectivity and tool-execution substrate for reaching robots deployed inside customer networks you don't own and can't configure. It is not a fleet management platform, not a robot autonomy framework, and not a SaaS product.

## Architecture

Three layers:

1. **Connectivity (Layer 1)** — libp2p peer identity, secure channels, transports (TCP + QUIC), circuit relay v2, NAT traversal (DCUtR hole-punching). Robots dial out; operators reach robots via a shared relay. Inbound connectivity is never assumed.

2. **Tool Execution (Layer 2)** — WASM component runtime (Wasmtime). Signed, sandboxed, versioned tools with explicit capability declarations. No ambient authority. Memory limits, CPU budgets, and wall-clock deadlines enforced per-component.

3. **Native Brokers (Layer 3)** — Privileged host processes that mediate WASM capability access to ROS 2 topics/services/parameters, filesystem, system logs, and diagnostics. Pattern-based access gating with default-deny policy.

## Quick start

```bash
# Build from source (Rust 1.85+)
cargo install --path crates/gang-cli

# Run the self-contained demo — no Docker, no ROS 2, no external dependencies
gang demo

# Or run a network archetype test scenario (requires Docker)
gang test-archetype open-warehouse
```

See [docs/QUICKSTART.md](docs/QUICKSTART.md) for a detailed walkthrough.

## Workspace structure

```
ganglion/
├── crates/
│   ├── gang-core/                  # Core types: identity, messages, policy, audit, manifests
│   ├── gang-libp2p/                # libp2p transport adapter (TCP, QUIC, relay, DCUtR)
│   ├── gang-wasm-host/             # Wasmtime component runtime (WIP)
│   ├── gang-ros/                   # ROS 2 brokers, robot agent, diagnostics
│   ├── gang-cli/                   # `gang` CLI binary
│   └── gang-capability-diagnostics/ # Reference WASM capability (WIP)
├── test-harness/                   # Docker-compose network archetype scenarios
│   ├── open-warehouse/             # Flat L2, direct connectivity
│   ├── nat-office/                 # Consumer NAT, relay + DCUtR
│   ├── enterprise-dmz/             # VLAN isolation, TCP 443 only
│   └── mobile-cgnat/               # Symmetric NAT, CGNAT, jitter + loss
└── docs/
    ├── DesignSpec.md               # Full architectural design specification
    ├── IMPLEMENTATION.md           # Implementation plan and progress checklist
    ├── QUICKSTART.md               # 5-minute getting-started guide
    └── VALIDATION.md               # Test harness results and measurements
```

## CLI commands

```
gang identity show              # Show your PeerId and public key
gang identity generate          # Generate a new Ed25519 keypair
gang sign <wasm-path>           # Sign a WASM component, produce .manifest.cbor
gang agent --data-dir <path>    # Run a local robot agent
gang deploy <robot> <wasm>      # Deploy a signed capability to a robot
gang run <robot> <cap> [args]   # Invoke an installed capability
gang caps <robot>               # List capabilities installed on a robot
gang logs <robot>               # Stream robot logs
gang demo                       # Self-contained end-to-end demo
gang test-archetype <archetype> # Launch a Docker network scenario
gang list                       # List reachable robots
gang connect <robot>            # Establish a session with a robot
```

## Network archetypes

Ganglion is designed around five real-world network environments:

| Archetype | Characteristics | Ganglion strategy |
|-----------|----------------|-------------------|
| Open warehouse | Flat L2, no NAT | Direct TCP/QUIC |
| NAT'd office | Consumer NAT, no inbound | Relay + DCUtR hole-punch |
| Enterprise DMZ | VLAN isolation, port restrictions | Relay on TCP 443 |
| Regulated facility | Air-gapped, physical sneakernet | Offline signed bundles |
| Mobile/CGNAT | Symmetric NAT, IP rotation | Relay-only, reconnect logic |

## Design principles

1. **Protocol-agnostic core, opinionated defaults.** libp2p is the default transport, not the only valid one.
2. **Capability-bounded remote execution.** All remote operations use signed, sandboxed WASM components with explicit capability declarations.
3. **Outbound-initiated by default.** Robots dial out. Operators reach robots via a shared relay.
4. **Operability before novelty.** Every feature must be debuggable from a single-operator laptop.
5. **Honest OSS boundary.** The reference demonstrates correctness; commercial products provide durability and governance.

## Security model

- Ed25519 keypair identity with PeerId derivation (blake3 hash of public key)
- Noise protocol encrypted channels (libp2p)
- Signed WASM component manifests with trust store verification
- Default-deny policy engine with glob pattern matching
- Append-only audit logging with size-based rotation
- Symlink jail enforcement on filesystem access

## Building

```bash
# Prerequisites: Rust 1.85+, cargo
cargo build --release

# Run tests (42 tests across all crates)
cargo test

# Build for Docker test harness
docker compose -f test-harness/open-warehouse/docker-compose.yml build
```

## License

Apache-2.0
