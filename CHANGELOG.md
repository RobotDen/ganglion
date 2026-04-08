# Changelog

All notable changes to Ganglion will be documented in this file.

## [0.5.0] — 2026-04-23

### Security

- **Filesystem broker symlink jail bypass** (`gang-ros`): Write operations to new files now canonicalize the parent directory, preventing path traversal via `../` or symlinked parents (ADR-015).
- **RosList access control enforcement** (`gang-ros`): `RosList` now filters results through allowed patterns — components can only see topics/services/nodes matching their policy.
- **Rosbridge naming correction** (`gang-ros`): Renamed `rosbridge_available` → `ros2_available` and `check_rosbridge()` → `check_ros2_available()` to accurately describe the current implementation (ros2 CLI, not WebSocket rosbridge).

### Added

- **WASM-to-broker glue layer** (`gang-wasm-host`): New `imports` module registers async host functions for all 8 WIT capability interfaces on the Wasmtime linker. WASM components can now call broker operations through their declared WIT imports, completing the Layer 2 → Layer 3 bridge that was the project's central architectural gap. Includes Val extraction helpers and JSON serialization across the WASM boundary.
- **WASM execution path in robot agent** (`gang-ros`): `invoke_capability()` now attempts WASM execution when component bytes contain valid WASM (`\0asm` magic header). Falls back to direct broker invocation for non-WASM capabilities.
- **Reference diagnostic capability** (`gang-capability-diagnostics`): Full implementation with `DiagnosticReport` struct, `collect()` function, `format_report()` output, and 6 tests. Replaces the empty 2-line stub.
- **ROS broker operations** (`gang-ros`): `ServiceCall`, `ParamGet`, and `ParamSet` broker operations with structured rosbridge-protocol responses (ADR-016, ADR-017).
- **`param-set` WIT operation** (`gang-wasm-host`): Added to `ros-interface`; WIT package version bumped to `@0.5.0` (ADR-017).
- **`gang status` CLI command** (`gang-cli`): Reports version, identity, registry, available and WIP capabilities (ADR-018).
- **Real CID in `gang deploy`** (`gang-cli`): Deploy now computes actual CID from manifest bytes instead of using a hardcoded placeholder.
- **Capability loading on agent startup** (`gang-ros`): `load_installed_capabilities()` now scans the capabilities directory, deserializes manifests, verifies signatures and trust store, and logs warnings for failures.
- **62 new tests** across 5 crates. **188 total tests passing.**

### Changed

- `BrokerOperation` enum gains `ParamSet` variant.
- WIT package version `ganglion:capability@0.5.0`.
- Wasmtime engine configured with `async_support(true)` for correct async instantiation.
- `gang-ros` depends on `gang-wasm-host` for WASM execution path.

---

## [0.4.0] — 2026-04-23

### Added

- **Expanded capability interface** (`gang-core`, `gang-ros`): Three new WIT capability groups — `ganglion:process/spawn@1.0` (bounded subprocess invocation with command allowlist), `ganglion:network/probe@1.0` (ping, DNS, port check, traceroute), `ganglion:metrics/emit@1.0` (structured metric emission with ring buffer). Full Layer 3 broker implementations for all three.
- **Standard capability library**: Three Rust capability crates — `gang-capability-param-inspect` (parameter server snapshot with diff), `gang-capability-diagnostic-bundle` (v2 comprehensive diagnostics with automated health checks), `gang-capability-network-archetype` (v2 archetype detection with connectivity scoring and recommendations).
- **Capability registry** (`gang-core`): Content-addressed registry with publish, search (by name/description/tags), install, multi-version support, persist/reload. CLI commands: `gang registry search/install/publish/list/info`.
- **Community pathway**: `docs/CAPABILITY_AUTHOR_GUIDE.md` with language-specific guides for Rust, C++, Python, and Go/TinyGo. `gang capability scaffold <name> --language <lang>` generates project skeletons with Makefile, source template, and WIT directory.
- **Manifest schema v2.0**: Adds authoring language, description, tags, minimum Ganglion version, and schema version fields. v1.x manifests load via `#[serde(default)]` backward compatibility.
- **WIT interface v0.4.0** with `process-spawn`, `network-probe`, and `metrics-emit` interfaces.
- **36 new tests** across 6 crates. **126 total tests passing.**

### Changed

- `CapabilityGroup` enum gains `ProcessSpawn`, `NetworkProbe`, `MetricsEmit` variants.
- `BrokerOperation` enum gains process, network probe, and metric operations.
- Policy engine and permissive policy updated for all new capability groups.

### Breaking

- `ComponentManifest` struct has new required fields (schema_version, language, description, tags, min_ganglion_version) — all have `#[serde(default)]` for backward compatibility with v1.x manifests.

---

## [0.3.0] — 2026-04-23

### Added

- **Content-addressed artifact store** (`gang-core`): `ArtifactStore` with CIDv1 + Blake3 hashing, content-addressed filesystem layout (blobs/ and chunks/ directories with 4-char fanout), configurable chunk size (default 1 MB), block-level deduplication, LRU eviction with configurable size cap (default 1 GB), and JSON metadata index with persist/reload.
- **`Cid` type** (`gang-core`): Content identifier with `bafy` prefix + Blake3 hex hash. Supports `from_bytes`, `from_file`, `from_str`, and `verify`.
- **`ArtifactsPublish` capability group** (`gang-core`): New `CapabilityGroup::ArtifactsPublish` variant for content-addressed artifact publishing.
- **Broker operations** (`gang-core`): `ArtifactPublish` and `ArtifactExists` operations added to `BrokerOperation` enum.
- **WIT interface** (`gang-wasm-host`): `artifacts-publish` interface with `publish` and `exists` functions, WIT updated to v0.3.0.
- **CLI commands** (`gang-cli`): `gang fetch <cid>`, `gang push <path>`, `gang artifacts` for artifact management.
- **Policy engine** updated to handle `ArtifactsPublish` capability group.
- **10 new tests** for artifact store (CID determinism, dedup, chunking, LRU eviction, persist/reload, list). **70 total tests passing.**

---

## [0.2.0] — 2026-04-23

### Added

- **Happy-eyeballs transport selection** (`gang-core`): `TransportPreference` configuration with preferred transport order, dial timeout, and stagger delay. `dial_parallel` method on `TransportAdapter` attempts multiple transports concurrently with first-handshake-wins semantics.
- **Transport statistics** (`gang-core`, `gang-libp2p`): Per-peer `TransportStats` with transport type, RTT tracking, bytes sent/received, DCUtR upgrade status, uptime, and reconnection count. Ping events update RTT in real-time; DCUtR events update relay-to-direct transition state.
- **Network archetype detection** (`gang-ros`): Six network probes (internet connectivity, NAT status, multicast, outbound ports, DNS behavior, CGNAT detection) with classification logic mapping to five standard archetypes. Transport recommendations per archetype.
- **CLI commands**: `gang diagnose [robot]` for network archetype detection with recommendations. `gang transport-stats <robot>` for per-transport connection telemetry. `--prefer-transport` flag on `gang connect` for happy-eyeballs preference.
- **8 new tests** for archetype classification and probe execution (60 total).

### Changed

- `PeerConnection` in libp2p adapter tracks transport type, RTT history, DCUtR state, I/O counters, and reconnection count.

### Breaking

- `TransportAdapter` trait gains `dial_parallel` and `transport_stats` methods (with default implementations — existing impls compile unchanged).

---

## [0.1.0] — 2026-04-23

### Added

- **Core types** (`gang-core`): Ed25519 identity with PeerId derivation, CBOR message framing with varint length prefix, signed component manifests with trust store, default-deny policy engine with glob patterns, append-only audit logging with rotation.
- **libp2p transport** (`gang-libp2p`): Transport adapter with TCP, QUIC, Noise encryption, Yamux multiplexing, circuit relay v2, DCUtR hole-punching, Kademlia peer routing.
- **ROS 2 integration** (`gang-ros`): Diagnostics broker (system info, processes, network state), filesystem broker with symlink jail, log stream broker with source pattern filtering, ROS interface broker (topic subscribe, service call, param get).
- **Robot agent** (`gang-ros`): Deploy/invoke lifecycle with signature verification, trust store checking, policy evaluation, and audit logging.
- **CLI** (`gang`): Full command set — identity, sign, agent, deploy, run, caps, logs, demo, test-archetype, list, connect. Self-contained `gang demo` for zero-dependency end-to-end demonstration.
- **WASM runtime** (`gang-wasm-host`): Wasmtime component model with fuel metering, epoch-based wall-clock deadlines, capability declaration enforcement, and WIT interface definitions for all four v0.1 capability groups.
- **Test harness**: Four Docker-compose scenarios simulating open warehouse, NAT'd office, enterprise DMZ, and mobile/CGNAT network archetypes with tc/netem and iptables.
- **Documentation**: Design specification, implementation plan, quickstart guide, validation framework.
- **52 passing tests** across gang-core (23), gang-ros (19), and gang-wasm-host (10).
