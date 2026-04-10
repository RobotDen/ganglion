# Ganglion Architecture

This document describes Ganglion's three-layer architecture, crate responsibilities, data flows, and key design decisions.

## Overview

Ganglion solves a specific problem: reaching robots deployed inside customer networks you don't own, and executing bounded diagnostic/operational tools on them without persistent inbound connectivity. The architecture is organized into three layers, each with a clear trust boundary.

```
                    ┌────────────────────────────────────────┐
                    │            Operator Laptop              │
                    │   gang CLI  ───▶  libp2p dial-out      │
                    └──────────────────┬─────────────────────┘
                                       │
                               ┌───────▼───────┐
                               │  Relay Server  │
                               │  (circuit v2)  │
                               └───────┬───────┘
                                       │
                    ┌──────────────────▼─────────────────────┐
                    │            Robot Agent                   │
                    │  ┌─────────┐  ┌──────────┐  ┌────────┐ │
                    │  │ Layer 1 │  │ Layer 2  │  │Layer 3 │ │
                    │  │ libp2p  │  │  WASM    │  │Brokers │ │
                    │  │transport│──│ runtime  │──│(native)│ │
                    │  └─────────┘  └──────────┘  └────────┘ │
                    └────────────────────────────────────────┘
```

## Layer 1: Connectivity

**Crate:** `gang-libp2p` (with traits defined in `gang-core::transport`)

Layer 1 provides authenticated, encrypted, multiplexed connections between peers. It handles the hard part: establishing connections when neither party can accept inbound connections.

### Transport adapter trait

`gang-core` defines the `TransportAdapter` trait — a protocol-agnostic interface that any transport implementation can satisfy:

```rust
pub trait TransportAdapter: Send + Sync {
    async fn dial(&self, peer: &PeerId) -> Result<GanglionStream, TransportError>;
    async fn dial_parallel(&self, peer: &PeerId, pref: &TransportPreference)
        -> Result<DialResult, TransportError>;
    async fn listen(&self, protocol: ProtocolId, handler: StreamHandler)
        -> Result<(), TransportError>;
    fn local_peer_id(&self) -> PeerId;
    fn capabilities(&self) -> TransportCapabilities;
    fn events(&self) -> Pin<Box<dyn Stream<Item = TransportEvent> + Send>>;
    async fn announce_presence(&self, info: PresenceInfo) -> Result<(), TransportError>;
    async fn transport_stats(&self, peer: &PeerId) -> Option<TransportStats>;
    async fn shutdown(&self) -> Result<(), TransportError>;
}
```

### libp2p implementation

The default `Libp2pTransportAdapter` provides:

- **TCP + QUIC** dual-stack transports
- **Noise** protocol encryption
- **Yamux** stream multiplexing
- **Circuit relay v2** for NAT traversal (robots dial the relay; operators connect via the relay)
- **DCUtR** (Direct Connection Upgrade through Relay) for hole-punching after the relay connection is established
- **Kademlia** for peer routing and discovery
- **Happy-eyeballs** parallel dialing: attempt QUIC and TCP simultaneously, first handshake wins

### Protocols

All application traffic is multiplexed over three stream protocols:

| Protocol | Purpose |
|----------|---------|
| `/ganglion/control/1.0` | Capability deployment, invocation, presence, configuration |
| `/ganglion/tool/1.0` | Bidirectional stream between operator and invoked capability |
| `/ganglion/bulk/1.0` | High-volume artifact transfer (log bundles, rosbags, tarballs) |

### Happy-eyeballs transport selection

Added in v0.2. When dialing a peer, Ganglion attempts multiple transports in parallel with a configurable stagger delay (default 250ms). The first successful handshake wins; remaining attempts are cancelled.

```rust
TransportPreference {
    preferred_order: vec!["quic", "tcp"],
    dial_timeout: Duration::from_secs(30),
    stagger_delay: Duration::from_millis(250),
}
```

## Layer 2: Tool Execution

**Crate:** `gang-wasm-host`

Layer 2 runs signed WASM components in a sandboxed runtime. Components have no ambient authority — they can only interact with the host through explicitly declared capability interfaces.

### Runtime

- **Wasmtime** component model runtime
- **Fuel metering** — each component gets a fuel budget; execution traps when fuel is exhausted
- **Epoch-based wall-clock deadlines** — prevents components from hanging indefinitely
- **Capability declaration enforcement** — the component's declared capabilities must pass policy evaluation before loading

### WIT interfaces

Ganglion defines its host-guest contract using WASM Interface Types (WIT). The `ganglion-capability` world imports eight interfaces:

```wit
package ganglion:capability@0.5.0;

world ganglion-capability {
    import ros-interface;
    import logs-stream;
    import fs-bounded;
    import diagnostics-collect;
    import artifacts-publish;
    import process-spawn;
    import network-probe;
    import metrics-emit;
}
```

Each interface defines typed records and functions that the host implements. Capabilities call these functions; the host routes calls through the appropriate Layer 3 broker after policy checks.

### Component manifest

Every WASM component has a signed manifest (`.manifest.cbor`) that declares:

- Component name, version, description
- Author peer ID (Ed25519 public key)
- Blake3 hash of the `.wasm` binary
- Declared capability groups with their parameters
- Schema version (v2.0), language, tags, minimum Ganglion version
- Ed25519 signature over the entire manifest

The manifest is verified at deploy time: the signature must be valid, the author must be in the trust store, and every declared capability must pass the policy engine.

## Layer 3: Native Brokers

**Crate:** `gang-ros`

Brokers are native Rust processes that mediate between the sandboxed WASM runtime and privileged host resources. Each broker implements the `CapabilityBroker` trait:

```rust
pub trait CapabilityBroker: Send + Sync {
    async fn handle_request(&self, req: CapabilityRequest)
        -> Result<CapabilityResponse, BrokerError>;
    fn capability_group(&self) -> &str;
}
```

### Broker inventory

| Broker | Capability group | What it mediates |
|--------|-----------------|------------------|
| `RosBroker` | `ganglion:ros/interface` | Topic subscribe, service call, parameter get/set |
| `LogStreamBroker` | `ganglion:logs/stream` | Journald, syslog, ROS log file access |
| `FsBroker` | `ganglion:fs/bounded` | Filesystem read/write with symlink jail |
| `DiagnosticsBroker` | `ganglion:diagnostics/collect` | System info, process lists, network state |
| `ProcessBroker` | `ganglion:process/spawn` | Subprocess execution with command allowlist and timeout |
| `NetworkProbeBroker` | `ganglion:network/probe` | Ping, DNS lookup, TCP port check, traceroute |
| `MetricsBroker` | `ganglion:metrics/emit` | Structured metric emission with ring buffer |

### Security enforcement in brokers

Each broker enforces its own access controls:

- **FsBroker**: Symlink jail — all paths are resolved and checked against the jail root. Symlinks that escape the jail are rejected.
- **ProcessBroker**: Command allowlist with glob pattern matching. Only explicitly permitted commands can execute.
- **LogStreamBroker**: Source pattern filtering — only logs matching the declared patterns are returned.
- **RosBroker**: Topic/service/parameter patterns with read-only vs read-write access control.

## Robot Agent

**Module:** `gang-ros::agent`

The `RobotAgent` is the central process running on a deployed robot. It owns the lifecycle:

1. **Identity** — loads or generates an Ed25519 keypair
2. **Transport** — starts the libp2p adapter, dials the relay
3. **Brokers** — initializes all Layer 3 brokers
4. **Policy** — loads the policy file
5. **Deploy** — receives signed components from operators, verifies signatures, checks trust store, evaluates policy, stores to disk
6. **Invoke** — loads a WASM component into the runtime, wires declared capabilities to brokers, executes, records audit log

## Content-Addressed Artifact Store

**Module:** `gang-core::artifacts`

Added in v0.3. The artifact store uses CIDv1 identifiers with Blake3 hashing for content-addressed storage of log bundles, rosbags, diagnostic tarballs, and other large artifacts.

- **CID format**: `bafy` prefix + hex-encoded Blake3 hash
- **Storage layout**: `store_dir/chunks/<4-char-prefix>/<full-cid>`
- **Deduplication**: Block-level dedup via content addressing
- **Eviction**: LRU eviction when store exceeds configurable size cap (default 1 GB)
- **Chunking**: Configurable chunk size (default 1 MB)
- **Metadata index**: JSON file with persist/reload for artifact metadata

## Capability Registry

**Module:** `gang-core::registry`

Added in v0.4. A JSON-backed registry for publishing, discovering, and installing WASM capabilities.

- **Publish**: stores capability metadata with content-addressed references
- **Search**: by name, description substring, or tags
- **Multi-version**: multiple versions of the same capability can coexist
- **Install**: retrieves capability by name and optional version
- **Persist/reload**: JSON file at `~/.gang/registry.json`

## Policy Engine

**Module:** `gang-core::policy`

The policy engine is default-deny. Policy files are TOML and define:

- **Capability rules**: which capability groups are allowed, with glob patterns for fine-grained access (e.g., allow `/diagnostics/**` but deny `/cmd_vel`)
- **Peer rules**: which operators can deploy capabilities, optionally restricted to specific capability names
- **Access levels**: for ROS interface, maximum access level (read_only vs read_write)

Policy is evaluated at deploy time (before the component is stored) and at invoke time (before the component is loaded into the runtime).

## Audit Logging

**Module:** `gang-core::audit`

Every capability invocation produces an audit record written to a local append-only CBOR log:

- Operator peer ID
- Component name, version, hash
- Capabilities used during invocation
- Wall-clock timestamps (start, end)
- Exit status (success, failed, timeout, trapped, policy-denied)
- Per-capability I/O statistics (bytes in, bytes out)
- Size-based rotation (configurable max size)

## Network Archetype Detection

**Module:** `gang-ros::archetype`

Ganglion includes automated network classification that runs six probes and maps results to one of five standard archetypes. Each archetype has pre-built transport recommendations. See [NETWORK_ARCHETYPES.md](NETWORK_ARCHETYPES.md) for details.

## Crate dependency graph

```
gang-core (no dependencies on other gang crates)
  ├── gang-libp2p (depends on gang-core)
  ├── gang-wasm-host (depends on gang-core)
  ├── gang-ros (depends on gang-core)
  ├── gang-capability-diagnostics (depends on gang-core)
  ├── gang-capability-param-inspect (depends on gang-core)
  ├── gang-capability-diagnostic-bundle (depends on gang-core)
  └── gang-capability-network-archetype (depends on gang-core)
gang-cli (depends on gang-core, gang-libp2p, gang-wasm-host, gang-ros)
```

`gang-core` has zero dependencies on other workspace crates. All other crates depend on `gang-core` for shared types. The CLI crate ties everything together.
