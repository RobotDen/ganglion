# Ganglion: implementation plan and checklist

**Status:** Active — tracks implementation from v0.1 through v0.6
**Source of truth:** [DesignSpec.md](./DesignSpec.md)

### Progress

**v0.1** (tagged `v0.1.0`):

| Phase | Status | Tests | Commit |
|-------|--------|-------|--------|
| 1. Repo scaffolding & core traits | Done | 23 | `8355084` |
| 2. Connectivity layer (libp2p) | Done | — | `e4ed53d` |
| 3. WASM runtime (Layer 2) | Done | 10 | `5767545` |
| 4. Native brokers (Layer 3) | Done | 19 | `51ceb47` |
| 5. Reference capability | Done (native) | — | `99e8ac1` |
| 6. CLI completion | Done | — | `99e8ac1` |
| 7. Test harness | Done | — | `416a4fc` |
| 8. Documentation | Done | — | `15e2be4` |

**v0.2** (tagged `v0.2.0`):

| Phase | Status | Tests | Commit |
|-------|--------|-------|--------|
| 9. Transport breadth | Done | — | `1a2f05f` |
| 10. CLI + archetype detection | Done | 8 | `1a2f05f` |
| 11. v0.2 release | Done | — | `9770710` |

**v0.3** (tagged `v0.3.0`):

| Phase | Status | Tests | Commit |
|-------|--------|-------|--------|
| 12. Content-addressed artifact store | Done | 10 | `7acc3be` |
| 13. Rosbag slicing capability | Deferred | — | — |
| 14. v0.3 CLI + release | Done | — | `7acc3be` |

**v0.4** (tagged `v0.4.0`):

| Phase | Status | Tests | Commit |
|-------|--------|-------|--------|
| 15. Expanded capability interface | Done | 19 | `3dae251` |
| 16. Standard capability library | Done | 25 | `8864d80` |
| 17. Capability registry | Done | 10 | `70b9ffe` |
| 18. Community pathway | Done | — | `3e937bb` |
| 19. v0.4 release | Done | 2 | — |

**Total: 126 tests passing across 7 crates (45 core + 46 ros + 10 wasm-host + 8 param-inspect + 7 diag-bundle + 10 net-archetype).**

**v0.5** (tagged `v0.5.0`):

| Phase | Status | Tests | Commit |
|-------|--------|-------|--------|
| 20. WASM-to-broker glue layer | Done | 8 | — |
| 21. ROS broker operations (ServiceCall, ParamGet, ParamSet) | Done | 14 | — |
| 22. Filesystem broker symlink jail fix | Done | 5 | — |
| 23. RosList access control enforcement | Done | 2 | — |
| 24. Transport layer completion (request_response, relay server) | Done | 9 | — |
| 25. Reference diagnostic capability | Done | 6 | — |
| 26. gang status CLI command | Done | — | — |
| 27. Capability loading on agent startup | Done | 2 | — |
| 28. Docker test harness fixes | Done | — | — |
| 29. Bootstrap relay deployment config | Done | — | — |
| 30. ROS helper hardening (validation, timeouts) | Done | 16 | — |
| 31. v0.5 release | Done | — | — |

**Total: 188 tests passing across 9 crates (46 core + 84 ros + 18 wasm-host + 9 libp2p + 6 diagnostics + 8 param-inspect + 7 diag-bundle + 10 net-archetype).**

**v0.6** ([ADR-020](adr/ADR-020-remote-dispatch-and-e2e-test.md)):

| Phase | Status | Tests | Commit |
|-------|--------|-------|--------|
| 32. Robot agent serve loop (listen on /ganglion/control/1.0) | Done | 188 | b729630 |
| 33. Agent CLI startup with transport (-r flag, dial relay) | Done | 188 | b729630 |
| 34. Peer registry CLI (gang peer add/remove/list/show/rename) | Done | 188 | 05810d7 |
| 35. Operator remote dispatch (name/prefix/peer-id resolution) | Done | 188 | 05810d7 |
| 36. SSH-style identity verification (TOFU, key-change warning) | Done | 188 | f00e329 |
| 37. Operator config file (~/.gang/config.toml) | Done | 188 | 42a8f63 |
| 38. Shell completions (bash, zsh, fish, elvish, powershell) | Done | 188 | 05810d7 |
| 39. Reference WASM component build (diagnostics → .wasm) | Done | 188 | b1d6099 |
| 40. E2E Docker test scenario (relay + robot + operator) | Done | 188 | b1d6099 |
| 41. v0.6 release | Done | 188 | e89167a |

**Scope:** Connect the existing transport infrastructure (`ControlMessage`, `GanglionCodec`, `handle_rpc_message()`, `PeerRegistry`, `TrustStore`) to the CLI and robot agent. Named peers, abbreviated peer ID matching (Docker-style), SSH-style host key verification, and a config file eliminate verbose flags. `gang deploy warehouse-bot diagnostics.wasm` sends a capability over a relay circuit to a named remote robot and returns the result. Validate with a Docker e2e scenario. See ADR-020 for detailed design.

---

## Repository layout

Four independent repos, each a Cargo workspace (or single crate where appropriate):

| Repo | Contents | Primary crate(s) |
|------|----------|-------------------|
| `tafy-labs/ganglion` | Core library, spec, docs, test harness | `gang-core`, `gang-wasm-host` |
| `tafy-labs/ganglion-libp2p` | libp2p transport adapter | `gang-libp2p` |
| `tafy-labs/ganglion-ros` | ROS 2 integration broker | `gang-ros` |
| `tafy-labs/gang` | CLI binary | `gang` |

**Rationale for keeping `gang-wasm-host` in the core repo:** the WASM host is tightly coupled to `gang-core`'s policy engine and audit logger. Separate repo adds cross-repo version coordination pain with no independence benefit.

### Cross-repo dependency graph

```
gang (CLI)
├── gang-core
├── gang-libp2p
├── gang-ros (optional, for local testing)
└── gang-wasm-host (via gang-core)

gang-ros
├── gang-core
└── gang-libp2p

gang-libp2p
└── gang-core (TransportAdapter trait)

gang-core
└── gang-wasm-host (inline)
```

---

## Dependency selections

These are the recommended starting points. Pin exact versions in `Cargo.lock`; update deliberately.

| Dependency | Crate | Purpose | Notes |
|------------|-------|---------|-------|
| `rust-libp2p` | `gang-libp2p` | Peer identity, transports, relay, DCUtR | Use latest 0.54.x+. Needed behaviors: identify, ping, kademlia, relay-v2, dcutr, noise, yamux, tcp, quic |
| `wasmtime` | `gang-wasm-host` | WASM component runtime | Use latest stable (v19+). Component model support required. |
| `wit-bindgen` | `gang-wasm-host` + capabilities | WIT binding generation | Match wasmtime version |
| `wasm-tools` | build tooling | Component production from wasm32-wasip2 modules | CLI tool, not a library dep |
| `ciborium` | `gang-core` | CBOR encoding/decoding | Lightweight, well-maintained |
| `ed25519-dalek` | `gang-core` | Keypair generation, signing, verification | Already used by libp2p internally |
| `rclrs` | `gang-ros` | ROS 2 Rust client bindings | Target Jazzy Jalisco (latest LTS). Verify `rclrs` compatibility. |
| `rosbridge_suite` | `gang-ros` (runtime dep) | WebSocket bridge to ROS 2 graph | System package, not Cargo dep |
| `beetswap` | `gang-core` (v0.3) | Bitswap block exchange | Evaluate at v0.3 planning time |
| `blake3` | `gang-core` (v0.3) | Content hashing for CIDs | |
| `clap` | `gang` | CLI argument parsing | |
| `tokio` | all | Async runtime | Use `tokio` throughout; `rust-libp2p` is tokio-native |
| `tracing` | all | Structured logging | With `tracing-subscriber` for CLI output |
| `serde` + `serde_cbor` | all | Serialization | CBOR for wire, serde for internal |
| `sqlx` or `rusqlite` | `gang-core` (v0.3) | SQLite for content store metadata | Evaluate at v0.3 |

---

## v0.1 — Reference control plane

### Phase 1: Repo scaffolding and core traits

Set up all four repos with CI, licensing, and the foundational type system.

**1.1 — `tafy-labs/ganglion` repo init**
- [ ] Create repo with Apache-2.0 license, README, `.gitignore`
- [ ] Cargo workspace with two members: `gang-core`, `gang-wasm-host`
- [ ] Copy `docs/DesignSpec.md` into repo
- [ ] Add `docs/decision-flowchart.svg` placeholder
- [ ] Set up GitHub Actions CI: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`
- [ ] Add `CONTRIBUTING.md` with DCO sign-off requirement
- [ ] Add `rust-toolchain.toml` pinning stable + `wasm32-wasip2` target

**1.2 — `gang-core` foundational types**
- [ ] `PeerId` type (re-export from `libp2p-identity` or define wrapper)
- [ ] `Role` enum: `RobotAgent`, `Operator`, `Relay`
- [ ] `TransportAdapter` trait (§4.2 of spec):
  ```rust
  trait TransportAdapter: Send + Sync {
      async fn dial(&self, peer: PeerId) -> Result<Stream>;
      async fn listen(&self, protocol: ProtocolId, handler: StreamHandler) -> Result<()>;
      fn local_peer_id(&self) -> PeerId;
      fn capabilities(&self) -> TransportCapabilities;
  }
  ```
- [ ] `TransportCapabilities` struct (relay support, hole-punching, direct dial, encryption guarantees)
- [ ] `ProtocolId` type for stream protocol identifiers
- [ ] `StreamHandler` trait for protocol handlers
- [ ] `Stream` abstraction over async read/write with protocol metadata
- [ ] Stream protocol constants: `/ganglion/control/1.0`, `/ganglion/tool/1.0`, `/ganglion/bulk/1.0`
- [ ] Error types: `GanglionError`, `TransportError`, `PolicyError`, `AuditError`
- [ ] Unit tests for all type construction and validation

**1.3 — `tafy-labs/ganglion-libp2p` repo init**
- [ ] Create repo with Apache-2.0 license
- [ ] Single crate, depends on `gang-core` (git dependency initially)
- [ ] GitHub Actions CI mirroring core repo
- [ ] Stub `TransportAdapter` impl that compiles but panics

**1.4 — `tafy-labs/ganglion-ros` repo init**
- [ ] Create repo with Apache-2.0 license
- [ ] Single crate, depends on `gang-core` and `gang-libp2p`
- [ ] CI with ROS 2 Jazzy container for testing
- [ ] Stub broker traits

**1.5 — `tafy-labs/gang` repo init**
- [ ] Create repo with Apache-2.0 license
- [ ] Binary crate with `clap` CLI skeleton
- [ ] Subcommands stubbed: `connect`, `list`, `caps`, `deploy`, `run`, `logs`, `test-archetype`
- [ ] CI: build + `--help` smoke test
- [ ] `cargo install` works from the repo

### Phase 2: Connectivity layer (Layer 1)

The libp2p transport adapter — the foundation everything else builds on.

**2.1 — Peer identity and keypair management**
- [ ] Ed25519 keypair generation and persistence (`~/.gang/identity.key` or configurable path)
- [ ] Load-or-generate on startup
- [ ] Peer ID derivation from public key
- [ ] Human-readable name registry (local JSON file mapping names ↔ peer IDs)
- [ ] `gang identity show` CLI command (print local peer ID)
- [ ] `gang identity generate` CLI command (create new keypair)
- [ ] Unit tests: generate, persist, reload, derive peer ID

**2.2 — libp2p node setup (`gang-libp2p`)**
- [ ] Configure `rust-libp2p` swarm with:
  - Noise protocol for encryption
  - Yamux for multiplexing
  - TCP transport
  - QUIC transport
  - Identify behavior
  - Ping behavior
  - Kademlia for peer routing (bootstrap node support)
  - Circuit relay v2 client behavior
  - DCUtR behavior
- [ ] Implement `TransportAdapter` trait for the configured swarm
- [ ] `dial()`: connect to peer via relay or direct
- [ ] `listen()`: register protocol handler
- [ ] `local_peer_id()`: return derived peer ID
- [ ] `capabilities()`: report supported transports and features
- [ ] Configurable listen addresses (multiaddr)
- [ ] Integration test: two nodes connect directly over localhost TCP

**2.3 — Relay support**
- [ ] Relay mode: a Ganglion node running as circuit relay v2 server
- [ ] Robot agent startup sequence (§3.3):
  1. Load/generate keypair
  2. Dial configured relay(s) outbound
  3. Register with relay, remain available for reservation
  4. Send signed presence message to authorized operators
- [ ] Operator connection flow: connect to relay, request circuit to robot peer ID
- [ ] DCUtR upgrade: attempt direct connection after relay-mediated handshake
- [ ] Reconnection logic: exponential backoff on relay disconnect, re-register on reconnect
- [ ] Integration test: operator reaches robot through relay (three-node test)
- [ ] Integration test: DCUtR upgrade succeeds on permissive NAT (simulated)

**2.4 — Stream protocols**
- [ ] Length-prefixed framing implementation (varint length prefix + CBOR payload)
- [ ] `/ganglion/control/1.0` protocol handler:
  - Presence messages (signed, with peer metadata)
  - Capability deployment messages
  - Capability invocation requests/responses
  - Configuration messages
- [ ] `/ganglion/tool/1.0` protocol handler:
  - Bidirectional byte stream between operator and invoked capability
  - Stream lifecycle: open, data, close, error
- [ ] `/ganglion/bulk/1.0` protocol handler:
  - High-volume transfer with flow control
  - Progress reporting
- [ ] CBOR message schemas for each protocol (using `ciborium`)
- [ ] Message validation and version negotiation
- [ ] Unit tests: frame encoding/decoding round-trips
- [ ] Integration test: control message exchange over a live connection

### Phase 3: Tool execution layer (Layer 2)

The WASM runtime and capability system.

**3.1 — WIT interface definitions**
- [ ] Author WIT files for v0.1 capability groups:
  - `ganglion:ros/interface@1.0` — topic subscribe, service call, parameter get/set (with pattern gating)
  - `ganglion:logs/stream@1.0` — log source enumeration, filtered log streaming
  - `ganglion:fs/bounded@1.0` — path-gated read/write/list
  - `ganglion:diagnostics/collect@1.0` — system info, process list, network state
- [ ] WIT world definition combining all capability groups
- [ ] Generate Rust host bindings via `wit-bindgen`
- [ ] Generate Rust guest bindings for capability authors
- [ ] Validate WIT files compile cleanly with `wasm-tools`

**3.2 — WASM runtime (`gang-wasm-host`)**
- [ ] Wasmtime engine configuration:
  - Component model enabled
  - Fuel metering for CPU budgets
  - Memory limits from manifest
  - Epoch interruption for wall-clock deadlines
- [ ] Component loader: read `.wasm` file, instantiate with capability bindings
- [ ] Capability enforcement: only link WIT imports that the manifest declares
- [ ] Undeclared capability calls trap immediately
- [ ] Resource limits: enforce max memory, CPU fuel, wall-clock deadline from manifest
- [ ] Stdout/stderr capture and forwarding to operator stream
- [ ] Clean shutdown on timeout or fuel exhaustion
- [ ] Unit test: load a trivial WASM component, invoke, get result
- [ ] Unit test: undeclared capability traps
- [ ] Unit test: fuel exhaustion terminates cleanly
- [ ] Unit test: memory limit enforced

**3.3 — Signing and manifest (§3.6)**
- [ ] Manifest schema (CBOR):
  - Component name, version
  - Declared capabilities (list of WIT interface references with version)
  - Author peer ID
  - Signature (Ed25519 over component bytes + manifest bytes)
  - Optional: max memory, CPU budget, wall-clock deadline
- [ ] Manifest serialization/deserialization
- [ ] Signing workflow: `gang sign <wasm-path> --key <keyfile>` produces `<name>.manifest.cbor`
- [ ] Verification: check signature against author's public key
- [ ] Trust store: local file listing trusted peer IDs
- [ ] Rejection logic: unsigned → reject; untrusted signer → reject; capabilities exceed policy → reject
- [ ] Unit tests: sign, verify, tamper-detect, trust store lookup

**3.4 — Policy engine**
- [ ] Policy definition format (TOML or CBOR):
  - Per-capability-group rules: allowed topic patterns, path patterns, log sources
  - Per-peer rules: which operators can deploy which capabilities
  - Default-deny semantics
- [ ] Policy evaluation at component load time
- [ ] Policy file location and loading (robot-local, e.g., `/etc/gang/policy.toml`)
- [ ] Policy violation produces structured error returned to operator
- [ ] Unit tests: policy permits, policy denies, pattern matching

**3.5 — Audit logger (§3.7)**
- [ ] Audit record schema:
  - Invoking operator peer ID
  - Component name, version, hash (Blake3 or SHA-256)
  - Declared capabilities used
  - Wall-clock start/end timestamps
  - Exit status
  - Bytes in/out per capability
- [ ] Append-only local log (one file per robot, newline-delimited CBOR)
- [ ] Log rotation by size (configurable)
- [ ] `gang audit <robot>` CLI command to retrieve and display audit records
- [ ] Unit test: write record, read back, verify integrity

### Phase 4: Native privileged layer (Layer 3)

Brokers that mediate between WASM capabilities and privileged resources.

**4.1 — ROS 2 interface broker (`gang-ros`)**
- [ ] Broker trait definition in `gang-core`:
  ```rust
  trait CapabilityBroker: Send + Sync {
      async fn handle_request(&self, req: CapabilityRequest) -> Result<CapabilityResponse>;
  }
  ```
- [ ] `RosInterfaceBroker` implementation:
  - Topic subscription (read-only): subscribe to topics matching declared patterns, forward serialized messages
  - Topic publish (read-write): publish to topics matching declared patterns
  - Service call: call services matching declared patterns
  - Parameter operations: get/set parameters matching declared patterns
- [ ] Pattern-based access control: broker checks each operation against the capability's declared patterns before executing
- [ ] Integration with `rclrs`: create ROS 2 node, manage subscriptions/publishers/clients
- [ ] Rosbridge WebSocket fallback: if `rclrs` direct integration is unavailable, bridge via rosbridge
- [ ] Integration test: WASM capability reads a ROS topic through the broker (requires ROS 2 environment)

**4.2 — Log stream broker**
- [ ] `LogStreamBroker` implementation:
  - Enumerate available log sources (journald, ROS log files, custom)
  - Stream filtered log lines matching source patterns
  - Backpressure handling
- [ ] journald integration via `libsystemd` or subprocess
- [ ] ROS log file tailing (`/rosout` topic subscription or file-based)
- [ ] Integration test: capability requests logs, broker streams them

**4.3 — Filesystem broker**
- [ ] `FsBroker` implementation:
  - Path-pattern-gated read, write, list, stat
  - Explicit flags per path pattern: read/write/execute
  - Reject any path outside declared patterns
  - Symlink resolution and jail enforcement
- [ ] Unit test: access within pattern succeeds, outside fails

**4.4 — Diagnostics broker**
- [ ] `DiagnosticsBroker` implementation:
  - System info collection (hostname, OS, kernel, uptime, CPU, memory, disk)
  - Process list (name, PID, CPU%, memory%)
  - Network state (interfaces, addresses, routes, connections)
- [ ] Cross-platform considerations (Linux primary, macOS for dev)
- [ ] Unit test: diagnostics collection returns structured data

### Phase 5: Reference capability

**5.1 — `gang-capability-diagnostics`**
- [ ] New Rust crate targeting `wasm32-wasip2`
- [ ] WIT imports: `ganglion:ros/interface@1.0` (read-only), `ganglion:logs/stream@1.0`, `ganglion:diagnostics/collect@1.0`
- [ ] Implementation:
  - Collect system info via diagnostics broker
  - Collect ROS node list, active topics, `/diagnostics` aggregate via ROS broker
  - Collect last N seconds of `/rosout` via log broker
  - Assemble CBOR bundle
  - Return bundle to operator
- [ ] Build pipeline: `cargo build --target wasm32-wasip2` → `wasm-tools component new` → signed component
- [ ] Manifest with declared capabilities
- [ ] Sign with test keypair
- [ ] End-to-end test: deploy to simulated robot, invoke, receive bundle

### Phase 6: CLI completion

**6.1 — `gang` CLI commands (v0.1 surface)**
- [ ] `gang connect <robot>` — establish session via relay, print connection status
- [ ] `gang list` — discover reachable robots (via relay presence messages), display table
- [ ] `gang caps <robot>` — list installed capabilities on a robot
- [ ] `gang deploy <robot> <wasm-path>` — transfer signed component to robot, robot verifies and installs
- [ ] `gang run <robot> <cap-name> [args]` — invoke capability, stream output to terminal
- [ ] `gang logs <robot> [--follow]` — stream robot logs via log broker
- [ ] `gang sign <wasm-path> --key <keyfile>` — sign a capability (may live here or in a separate tool)
- [ ] `gang identity show` / `gang identity generate`
- [ ] `gang test-archetype <archetype>` — launch Docker-compose scenario (see Phase 7)
- [ ] Structured output mode (`--json`) for all commands
- [ ] Error messages that are actionable (not just "connection failed" — say why and what to try)
- [ ] `--help` text for every command and subcommand

### Phase 7: Test harness

**7.1 — Docker-compose test scenarios**
- [ ] Base container image: Ubuntu + ROS 2 Jazzy + `gang-ros` + simulated robot (minimal talker/listener nodes)
- [ ] Network simulation tooling: `tc`/`netem` for latency/jitter, `iptables` for firewall rules
- [ ] Scenario 1 — Open warehouse: flat L2, no NAT, no egress controls
- [ ] Scenario 2 — NAT'd office: single consumer NAT, no inbound, DHCP rotation
- [ ] Scenario 3 — Enterprise DMZ: VLAN isolation, restricted outbound ports, TLS inspection proxy
- [ ] Scenario 4 — Mobile/CGNAT: symmetric NAT, IP rotation, intermittent connectivity
- [ ] Each scenario measures:
  - Time to first successful connection
  - Steady-state RTT for control messages
  - Throughput for bulk transfers
  - Reconnection time after transient failure
  - DCUtR upgrade success/failure
- [ ] Results capture script → `docs/VALIDATION.md`
- [ ] `gang test-archetype` CLI integration: runs the Docker-compose scenario and prints results

### Phase 8: Documentation and release

**8.1 — v0.1 documentation**
- [ ] `README.md` for each repo (purpose, quickstart, build instructions, license)
- [ ] `docs/DesignSpec.md` — already exists, verify current
- [ ] `docs/VALIDATION.md` — test harness results with measured numbers
- [ ] `docs/decision-flowchart.svg` — one-page architectural selection flowchart
- [ ] `docs/QUICKSTART.md` — 5-minute path from clone to running `gang test-archetype`
- [ ] Inline rustdoc on all public APIs
- [ ] `CHANGELOG.md` for each repo

**8.2 — Release**
- [ ] All four repos public on GitHub under `tafy-labs/`
- [ ] `cargo install gang` works from a clean environment
- [ ] Git tags: `v0.1.0` on all repos
- [ ] GitHub releases with changelogs
- [ ] Blog post checklist complete (see `content/blog-drafts/2026-04-24-*`)

---

## v0.2 — Transport breadth

### Phase 9: Additional transports

**9.1 — WebTransport support**
- [ ] Add `quinn`-based WebTransport adapter to `gang-libp2p`
- [ ] Operator-side: connect over HTTPS/443 when other transports are blocked
- [ ] Integration test: operator behind HTTPS-only egress reaches robot via WebTransport

**9.2 — WebRTC transport**
- [ ] Add rust-libp2p WebRTC transport to `gang-libp2p`
- [ ] Expose through Rust API (browser UI is a later item)
- [ ] Integration test: WebRTC connection establishment

**9.3 — Happy-eyeballs transport selection**
- [ ] `TransportAdapter` trait update: add `dial_parallel` method
- [ ] Parallel connection attempts across QUIC, WebTransport, TCP
- [ ] First successful handshake wins; cancel remaining
- [ ] Configurable preference order
- [ ] `--prefer-transport` CLI flag on `gang connect`
- [ ] Integration test: happy-eyeballs selects fastest transport

**9.4 — Noise tuning for high-latency**
- [ ] Noise handshake parameter tuning for satellite/mobile links
- [ ] Configurable handshake timeout
- [ ] Test with simulated 500ms+ RTT

### Phase 10: v0.2 CLI and capability additions

**10.1 — New CLI commands**
- [ ] `gang diagnose <robot>` — run network-archetype-detector capability, report transport path, relay hops, constraints
- [ ] `gang transport-stats <robot>` — per-transport statistics for active connection

**10.2 — `gang-capability-network-archetype`**
- [ ] WASM capability: probe local network from robot side
- [ ] Probes: outbound connectivity, MTU discovery, multicast reachability, DNS behavior, STUN queries
- [ ] Classification output: machine-readable schema mapping to the five archetypes
- [ ] Recommendations based on detected archetype

### Phase 11: v0.2 release

- [ ] Breaking change documentation: `TransportAdapter::dial_parallel`, `gang connect` default behavior change
- [ ] Update `VALIDATION.md` with transport-specific measurements
- [ ] Update `CHANGELOG.md` across all repos
- [ ] Git tags: `v0.2.0`

---

## v0.3 — Content-addressed forensic artifact distribution

### Phase 12: Content-addressed storage

**12.1 — `gang-artifacts` module (in `gang-core` or new crate)**
- [ ] CID v1 + Blake3 hashing implementation
- [ ] Content-addressed filesystem layout for local store
- [ ] SQLite index for artifact metadata (CID, size, origin peer, timestamp)
- [ ] Configurable size cap and LRU eviction
- [ ] Chunking for large artifacts
- [ ] Deduplication at the block level
- [ ] Unit tests: store, retrieve, evict, dedup

**12.2 — Block exchange (bitswap)**
- [ ] Integrate `beetswap` (or equivalent) for block exchange over libp2p
- [ ] Resumable transfers: interrupted transfer resumes from last block
- [ ] Peer discovery for cached artifacts (operator-to-operator fetch)
- [ ] Integration test: transfer large artifact, interrupt, resume

**12.3 — Capability integration**
- [ ] New WIT capability group: `ganglion:artifacts/publish@1.0`
- [ ] WASM capabilities can publish byte streams → CID returned
- [ ] Diagnostics capability result format updated: `artifacts` field with CIDs for large attachments
- [ ] Manifest schema v1.1: optional `artifact-capabilities` declarations

### Phase 13: Rosbag slicing

**13.1 — `gang-capability-rosbag-slice`**
- [ ] WASM capability (Rust): capture time-bounded rosbag slice
- [ ] Parameters: `--start`, `--end`, `--topics` (comma-separated topic filters)
- [ ] Integration with `rosbag2` storage format
- [ ] Produce content-addressed rosbag2 bundle, return CID
- [ ] Output is a valid rosbag2 that replays locally
- [ ] End-to-end test: slice, fetch, replay

### Phase 14: v0.3 CLI and release

**14.1 — New CLI commands**
- [ ] `gang fetch <cid>` — retrieve artifact from any reachable peer
- [ ] `gang push <path>` — publish local file to content store, announce CID
- [ ] `gang artifacts list` — list locally-stored artifacts (CID, size, origin)

**14.2 — Release**
- [ ] Breaking change documentation: diagnostics result format, manifest v1.1
- [ ] Update `VALIDATION.md` with artifact transfer benchmarks
- [ ] `CHANGELOG.md` updates
- [ ] Git tags: `v0.3.0`

---

## v0.4 — Capability standard library

### Phase 15: Expanded capability interface

**15.1 — New WIT capability groups**
- [ ] `ganglion:process/spawn@1.0` — bounded subprocess invocation, captured stdio, resource limits
- [ ] `ganglion:network/probe@1.0` — structured network probing primitives
- [ ] `ganglion:metrics/emit@1.0` — structured metric emission from capabilities to operator
- [ ] Process broker implementation in Layer 3 (allowlist enforcement, resource limits)
- [ ] Network probe broker implementation
- [ ] Metrics broker implementation

### Phase 16: Standard capability library

**16.1 — Python capability: `gang-capability-log-normalize`**
- [ ] Set up `componentize-py` build pipeline
- [ ] Implement log format conversion (systemd, ROS, custom → structured normalized format)
- [ ] WIT bindings for Python
- [ ] Build → component → sign
- [ ] Test: deploy, invoke with mixed log sources, verify normalized output

**16.2 — C++ capability: `gang-capability-topic-echo`**
- [ ] Set up `wasi-sdk` + `wit-bindgen` C++ build pipeline
- [ ] Subscribe to specified ROS topics, stream serialized messages to operator
- [ ] Optional decimation (every Nth message)
- [ ] Build → component → sign
- [ ] Test: deploy, invoke, verify streamed topic data

**16.3 — Rust capabilities**
- [ ] `gang-capability-param-inspect` — parameter server snapshot, optional diff against reference
- [ ] `gang-capability-diagnostic-bundle` — v2 diagnostics (adds journald, dmesg, systemd units, ROS node graph, network state)
- [ ] `gang-capability-network-archetype` v2 — adds recommendation output

**16.4 — Go capability: `gang-capability-canary-probe`**
- [ ] Set up TinyGo → WASM component build pipeline
- [ ] Quick health check: is the robot responsive, basic vitals
- [ ] Designed for fleet-scale polling
- [ ] Build → component → sign
- [ ] Test: deploy, invoke, verify response

### Phase 17: Capability registry

**17.1 — Registry infrastructure**
- [ ] `registry.ganglion.tafy.dev` — content-addressed registry
- [ ] Registry protocol: libp2p pubsub topic + content-addressed document graph
- [ ] Registry entries: capability manifests with signatures + CID for WASM component
- [ ] Fetch via v0.3 content-addressed layer
- [ ] No central database

**17.2 — CLI registry commands**
- [ ] `gang registry search <query>` — discover capabilities by name/keyword
- [ ] `gang registry install <name>` — fetch, verify signature, install locally
- [ ] `gang registry publish <wasm-path>` — publish signed capability to registry

### Phase 18: Community pathway

**18.1 — Capability author guide**
- [ ] `docs/CAPABILITY_AUTHOR_GUIDE.md` — how to build, sign, distribute
- [ ] Language-specific subsections: Rust, C++, Python, Go/TinyGo
- [ ] Worked examples for each language

**18.2 — Scaffolding CLI**
- [ ] `gang capability scaffold <name> --language=<rust|cpp|python|go>` — generate project skeleton
- [ ] Templates for each language with WIT bindings, build scripts, Makefile/justfile
- [ ] Template repos: `tafylabs/gang-capability-template-{rust,cpp,python,go}`

### Phase 19: v0.4 release

**19.1 — Breaking changes and release**
- [ ] Manifest schema v2.0: formalized capability group versioning, registry metadata, authoring language field
- [ ] Manifest v1.x backward compatibility with deprecation warning
- [ ] Update all documentation
- [ ] `CHANGELOG.md` updates
- [ ] Git tags: `v0.4.0`
- [ ] All six standard capabilities published to registry

---

## Build order and critical path

The critical path through v0.1 is:

```
Phase 1 (scaffolding)
  → Phase 2 (connectivity) — longest phase, highest risk
    → Phase 3 (WASM runtime) — can partially overlap with Phase 2
      → Phase 4 (brokers) — depends on both Phase 2 and 3
        → Phase 5 (reference capability) — depends on Phase 4
          → Phase 6 (CLI completion) — incrementally built throughout
            → Phase 7 (test harness) — depends on working end-to-end stack
              → Phase 8 (docs + release)
```

**Parallelism opportunities in v0.1:**
- Phase 1 (all repos) is fully parallel
- WIT authoring (3.1) can start as soon as Phase 1 completes, independent of Phase 2
- Policy engine (3.4) and audit logger (3.5) can be built while Phase 2 is in progress
- Broker trait definitions (4.1) can be designed during Phase 2
- CLI skeleton (6.1) can be built incrementally as features land
- Docker-compose scenarios (7.1) can be authored while Phase 2 is in progress

**Risk items:**
- `rclrs` compatibility with target ROS 2 distro — validate early in Phase 4.1
- `wasmtime` component model stability — validate early in Phase 3.2
- DCUtR reliability across real NAT types — may require fallback strategies in Phase 2.3
- `beetswap` maturity (v0.3) — evaluate alternatives during v0.2 cycle
- `componentize-py` and `wasi-sdk` C++ toolchain stability (v0.4) — validate with hello-world components early

---

## Architectural decisions log

Track decisions made during implementation that deviate from or clarify the spec.

| # | Decision | Rationale | Date |
|---|----------|-----------|------|
| 001 | [Monorepo workspace](adr/ADR-001-monorepo-workspace.md) | Single team, tight cross-crate iteration; 4-repo split from spec was unnecessary overhead | v0.1 |
| 002 | [Three-layer architecture](adr/ADR-002-three-layer-architecture.md) | Isolate connectivity, tool execution, and privileged operations for independent failure and testing | Design |
| 003 | [Ed25519 identity](adr/ADR-003-ed25519-identity.md) | Self-certifying identity, no CA infrastructure, libp2p-compatible | v0.1 |
| 004 | [Default-deny policy](adr/ADR-004-default-deny-policy.md) | New capability groups must be secure by default; customer network exposure risk | v0.1 |
| 005 | [Transport adapter trait](adr/ADR-005-transport-adapter-trait.md) | Protocol-agnostic core enables alternative transports without modifying gang-core | v0.1 |
| 006 | [WASM component model](adr/ADR-006-wasm-component-model.md) | Sandboxing + portability + resource bounding for operator-supplied tools | v0.1 |
| 007 | [Simplified WASM linking](adr/ADR-007-simplified-wasm-linking.md) | Defer full WIT-to-broker binding to focus on proving broker architecture | v0.1 |
| 008 | [WIT capability interfaces](adr/ADR-008-wit-capability-interfaces.md) | Component model's native IDL; typed, composable, multi-language bindings | v0.1 |
| 009 | [Outbound-initiated connectivity](adr/ADR-009-outbound-initiated.md) | Robots behind customer NAT/firewalls cannot accept inbound; relay-first design | Design |
| 010 | [Content-addressed artifacts](adr/ADR-010-content-addressed-artifacts.md) | Blake3+CIDv1 for dedup, integrity, IPFS interop, LRU eviction on edge | v0.3 |
| 011 | [Manifest schema v2](adr/ADR-011-manifest-schema-v2.md) | Backward-compatible schema evolution via serde defaults | v0.3 |
| 012 | [Network archetype detection](adr/ADR-012-network-archetype-detection.md) | Automated transport recommendation from probe results | v0.4 |
| 013 | [Rosbridge over rclrs](adr/ADR-013-rosbridge-over-rclrs.md) | No build-time ROS 2 dependency; works with any distro; CI-friendly | v0.1 |
| 014 | [Rust edition 2024](adr/ADR-014-rust-edition-2024.md) | Newest stable edition, project targets Rust 1.85+ | v0.4 |
| 015 | [Fix symlink jail writes](adr/ADR-015-fix-symlink-jail-writes.md) | Canonicalize parent for new-file writes to close traversal gap | v0.5 |
| 016 | [Implement ROS stubs](adr/ADR-016-implement-ros-stubs.md) | Complete service-call and param-get via rosbridge | v0.5 |
| 017 | [WIT param-set](adr/ADR-017-wit-param-set.md) | Add write symmetry for parameters; enable field tuning capabilities | v0.5 |
| 018 | [Document CLI stubs](adr/ADR-018-document-cli-stubs.md) | Mark WIP commands clearly; add gang status command | v0.5 |
| 019 | [ROS broker tests](adr/ADR-019-ros-broker-test-coverage.md) | Zero unit tests on RosBroker; cover check_access and handle_request | v0.5 |
| 020 | [Remote dispatch and e2e test](adr/ADR-020-remote-dispatch-and-e2e-test.md) | Connect transport infra to CLI/agent; validate with Docker e2e scenario | v0.6 (proposed) |

---

## Open questions from spec (§11)

These should be resolved via RFCs in `docs/rfc/` between v0.1 and v1.0:

- [ ] Browser-based operator UI: first-class in OSS or ecosystem concern?
- [ ] Capability registry namespace: single global or organization-scoped from the start?
- [ ] v1.0 stability commitments: which surfaces (stream protocols, WIT interfaces, CLI)?
- [ ] Additional language support beyond Rust/C++/Python/Go: C#/.NET, Java/JVM?
