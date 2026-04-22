# Ganglion — Claude Code Instructions

## Identity

Hostile-network reachability and sandboxed field tooling for ROS 2 robot fleets. Three-layer architecture: libp2p connectivity, WASM sandboxed tool execution, native capability brokers with default-deny policy.

Part of the Tafy Labs / RobotDen vault ecosystem. Ganglion is the open-source connectivity layer underneath FleetLink (the commercial implementation service). Wiki entity: `wiki/entities/ganglion.md`.

## Build & Run

```bash
cargo build                          # Debug build
cargo build --release                # Release build
cargo install --path crates/gang-cli # Install `gang` CLI to PATH
cargo test                           # Run all 240 tests
cargo clippy --all-targets           # Lint (CI runs with -Dwarnings)
cargo fmt --check                    # Check formatting
cargo doc --no-deps                  # Build rustdoc
```

### Git hooks

```bash
./scripts/setup-hooks.sh             # One-time: sets core.hooksPath to .githooks/
```

Pre-commit hook runs: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --quiet`.

### CLI quick start

```bash
gang demo                            # Self-contained end-to-end demo
gang identity show                   # Show peer ID
gang status                          # Version, identity, capabilities summary
gang diagnose                        # Detect network archetype
gang capability scaffold my-tool     # Generate capability skeleton
gang registry list                   # List registered capabilities
gang peer list                       # List registered peers
gang config show                     # Show operator configuration
gang completions bash                # Generate bash completions
```

### Docker test harness

```bash
gang test-archetype open-warehouse   # Flat L2
gang test-archetype nat-office       # Consumer NAT + DCUtR
gang test-archetype enterprise-dmz   # VLAN, TCP 443 only
gang test-archetype mobile-cgnat     # Symmetric NAT, jitter, loss
```

## Workspace (13 crates)

```
crates/
├── gang-core/                          # Core types: identity, messages, policy, audit,
│                                       # manifests, artifacts, registry. Zero internal deps.
├── gang-libp2p/                        # libp2p transport adapter (TCP, QUIC, relay, DCUtR)
├── gang-wasm-host/                     # Wasmtime component runtime + WIT interfaces
├── gang-ros/                           # ROS 2 brokers, robot agent, archetype detection
├── gang-cli/                           # `gang` CLI binary (ties everything together)
├── gang-capability-diagnostics/        # Basic system diagnostics
├── gang-capability-param-inspect/      # ROS 2 parameter snapshot + diff
├── gang-capability-diagnostic-bundle/  # Comprehensive diagnostics with health checks
├── gang-capability-network-archetype/  # Archetype detection with connectivity scoring
├── gang-capability-log-normalize/      # Log format normalization (journald, ROS 2, syslog)
├── gang-capability-topic-echo/         # ROS 2 topic capture with decimation
├── gang-capability-canary-probe/       # Fleet-scale health polling
└── gang-capability-rosbag-slice/       # Time-bounded rosbag2 slicing
```

### Dependency rule

`gang-core` has zero dependencies on other workspace crates. All other crates depend on `gang-core`. The CLI crate depends on everything.

## Architecture

### Three layers

1. **Layer 1 — Connectivity** (`gang-libp2p`): Ed25519 identity, Noise encryption, TCP+QUIC, circuit relay v2, DCUtR hole-punching, Kademlia, happy-eyeballs parallel dialing.
2. **Layer 2 — Tool Execution** (`gang-wasm-host`): Wasmtime component model. Signed WASM components with fuel metering, epoch-based deadlines, no ambient authority.
3. **Layer 3 — Native Brokers** (`gang-ros`): Host processes mediating WASM access to system resources. Each broker implements `CapabilityBroker` trait.

### Eight capability groups (WIT interfaces)

| Group | Broker | Key constraint |
|-------|--------|----------------|
| `ganglion:ros/interface` | `RosBroker` | Topic/service/param pattern matching, read-only vs read-write |
| `ganglion:logs/stream` | `LogStreamBroker` | Source pattern filtering |
| `ganglion:fs/bounded` | `FsBroker` | Symlink jail enforcement |
| `ganglion:diagnostics/collect` | `DiagnosticsBroker` | System info collection |
| `ganglion:artifacts/publish` | `ArtifactStore` | CIDv1 + Blake3, LRU eviction |
| `ganglion:process/spawn` | `ProcessBroker` | Command allowlist, wall-clock timeout |
| `ganglion:network/probe` | `NetworkProbeBroker` | Structured probing (no raw sockets) |
| `ganglion:metrics/emit` | `MetricsBroker` | Ring buffer, configurable retention |

All interfaces are part of `ganglion:capability@0.5.0`. Individual interfaces are not independently versioned.

### Five network archetypes

Open warehouse (direct QUIC), NAT'd office (relay + DCUtR), enterprise DMZ (relay on TCP 443), regulated facility (offline signed bundles), mobile/CGNAT (relay-only).

## Conventions

### Code style

- All code must pass `cargo fmt` (default rustfmt settings)
- All code must pass `cargo clippy --all-targets` with zero warnings (CI uses `RUSTFLAGS="-Dwarnings"`)
- Public types and functions should have doc comments
- Doc comments on separate concerns must be joined with `///` continuation, not separated by blank lines (clippy: `empty_line_after_doc_comment`)

### Error handling

- `thiserror` for library error types (defined in `gang-core::error`)
- `anyhow` in the CLI crate only
- Return `Result`, don't panic

### Testing

- Tests live in `#[cfg(test)] mod tests` blocks within each module
- Use `tempfile::TempDir` for filesystem tests
- Network tests use localhost or mock data
- Async tests use `#[tokio::test]`

### Adding a new broker

1. Add `BrokerOperation` variant → `gang-core::broker`
2. Add `CapabilityGroup` variant → `gang-core::capability`
3. Update policy engine → `gang-core::policy`
4. Add WIT interface → `gang-wasm-host/wit/ganglion.wit`
5. Implement broker → `gang-ros/src/<name>.rs`
6. Register module → `gang-ros/src/lib.rs`
7. Wire into `RobotAgent` → `gang-ros/src/agent.rs`

### Adding a new CLI command

1. Add variant to `Commands` enum → `gang-cli/src/main.rs`
2. Implement handler → `gang-cli/src/commands.rs`
3. Wire in the `match cli.command` block
4. Update `docs/CLI_REFERENCE.md`

### Commit messages

Imperative mood, describe what the commit does. First line under 72 characters. Body for details.

## Security model

- Ed25519 keypair identity, PeerId = `12D3-` + Blake3(pubkey)[..16]
- Signed component manifests verified against trust store at deploy time
- Default-deny policy engine with glob pattern matching
- WASM sandbox: fuel metering, epoch deadlines, no ambient authority
- Append-only CBOR audit log with size-based rotation
- Key files at `~/.gang/` — identity.key, peers.json, trusted_peers.json, registry.json

## Vault integration

- Wiki entity: `../../wiki/entities/ganglion.md`
- Solution context: `../../wiki/solutions/fleetlink.md` (Ganglion is FleetLink's OSS layer)
- Blog coverage: `../../content/blog-drafts/2026-04-24-reaching-robots-behind-customer-firewalls.md`
- Vault standards: inherits from `../../standards/global/`
