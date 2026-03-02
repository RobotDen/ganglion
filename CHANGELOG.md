# Changelog

All notable changes to Ganglion will be documented in this file.

## [Unreleased] — v0.1.0-dev

### Added

- **Core types** (`gang-core`): Ed25519 identity with PeerId derivation, CBOR message framing with varint length prefix, signed component manifests with trust store, default-deny policy engine with glob patterns, append-only audit logging with rotation.
- **libp2p transport** (`gang-libp2p`): Transport adapter with TCP, QUIC, Noise encryption, Yamux multiplexing, circuit relay v2, DCUtR hole-punching, Kademlia peer routing.
- **ROS 2 integration** (`gang-ros`): Diagnostics broker (system info, processes, network state), filesystem broker with symlink jail, log stream broker with source pattern filtering, ROS interface broker (topic subscribe, service call, param get).
- **Robot agent** (`gang-ros`): Deploy/invoke lifecycle with signature verification, trust store checking, policy evaluation, and audit logging.
- **CLI** (`gang`): Full command set — identity, sign, agent, deploy, run, caps, logs, demo, test-archetype, list, connect. Self-contained `gang demo` for zero-dependency end-to-end demonstration.
- **Test harness**: Four Docker-compose scenarios simulating open warehouse, NAT'd office, enterprise DMZ, and mobile/CGNAT network archetypes with tc/netem and iptables.
- **Documentation**: Design specification, implementation plan, quickstart guide, validation framework.
- **42 passing tests** across gang-core (23) and gang-ros (19).
