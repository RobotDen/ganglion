# Changelog

All notable changes to Ganglion will be documented in this file.

## [Unreleased] — v0.2.0-dev

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
