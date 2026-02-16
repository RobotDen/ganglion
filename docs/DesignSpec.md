# Ganglion: architectural design specification

**Status:** Draft for v0.1 release **Repository:** `tafylabs/ganglion` **License:** Apache-2.0 **Authors:** Tafy Labs **Last updated:** April 23, 2026

---

## 1. Purpose and scope

Ganglion is an open-source reference architecture and implementation for **hostile-network reachability and sandboxed field tooling for ROS 2 robot fleets**. It provides the primitives required for a fleet operator to reach, observe, and act on robots deployed inside customer networks the operator does not own and cannot configure.

Ganglion is not a fleet management platform. It is not a robot autonomy framework. It is not a SaaS product. It is the connectivity and tool-execution substrate on which those things can be built.

### 1.1 Non-goals

Ganglion does not attempt to:

- Replace ROS 2's internal middleware (DDS, Zenoh) for intra-robot or intra-LAN communication
- Provide a full teleoperation stack, low-latency video streaming, or a video-first operator UI
- Provide fleet orchestration, mission planning, or task assignment
- Provide multi-tenant isolation suitable for shared SaaS deployment
- Replace commercial vendors where their offering meets the user's needs

### 1.2 Design principles

Five principles govern every version and every component:

1. **Protocol-agnostic core, opinionated defaults.** The core specification defines adapter interfaces; libp2p is the recommended default transport but not the only valid one.
2. **Capability-bounded remote execution.** All remote operations on a robot occur through signed, sandboxed, versioned components with explicit capability declarations. No ambient authority.
3. **Outbound-initiated by default.** Robots dial out. Operators reach robots via a shared broker. Inbound connectivity is never assumed.
4. **Operability before novelty.** Every feature must be debuggable from a single-operator laptop with commodity tooling when the system fails.
5. **Honest OSS boundary.** The reference demonstrates correctness; commercial products built on top provide durability, governance, and enterprise fit. This boundary is explicit in every interface.

---

## 2. Architecture overview

Ganglion is a three-layer architecture.

```
┌──────────────────────────────────────────────────────────┐
│                   Operator Environment                   │
│   gang CLI  •  operator libraries  •  dashboards         │
└────────────────────────┬─────────────────────────────────┘
                         │ (ganglion stream protocol)
┌────────────────────────▼─────────────────────────────────┐
│              Connectivity Layer (Layer 1)                │
│   libp2p peer identity, secure channels, transports,     │
│   multiplexing, relay, NAT traversal (DCUtR, hole-punch) │
└────────────────────────┬─────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────┐
│              Tool Execution Layer (Layer 2)              │
│   WASM runtime on robot  •  signed components            │
│   WIT-defined capability interface  •  policy engine     │
└────────────────────────┬─────────────────────────────────┘
                         │ (capability requests)
┌────────────────────────▼─────────────────────────────────┐
│          Native Privileged Layer (Layer 3)               │
│   ROS 2 interface broker  •  hardware access broker      │
│   filesystem broker  •  process broker                   │
└──────────────────────────────────────────────────────────┘
```

### 2.1 Layer responsibilities

**Layer 1 — Connectivity (native).** Establishes and maintains peer-to-peer secure channels between operator environments and robot agents. Handles transport selection, NAT traversal, relay, reconnection, and peer identity. Runs as a native process on both operator machines and robots. Stateless with respect to robot application logic.

**Layer 2 — Tool Execution (WASM).** Hosts signed WebAssembly components shipped by operators. Each component runs in a sandbox with declared capabilities. Components cannot directly touch the operating system, ROS 2 topics, device files, or the network — they request access via Layer 3 brokers according to their declared capabilities.

**Layer 3 — Privileged Operations (native).** A small set of native brokers that mediate access to privileged resources — ROS 2 interfaces, hardware devices, filesystem regions, process control, and packet capture. Brokers enforce policy and produce audit records.

### 2.2 Why three layers

The split is not aesthetic. It reflects three distinct operational truths:

- Connectivity must be maximally reliable and minimally interesting. The mesh needs to stay up when everything else is broken. Native.
- Tools change much faster than firmware and come from less-trusted sources. They need sandboxing, versioning, and signature verification. WASM.
- Some operations must touch hardware, the kernel, or ROS 2's internals directly. These cannot be safely sandboxed. Native, narrow, and auditable.

Collapsing any two layers increases blast radius or reduces flexibility. Splitting further adds complexity without benefit.

---

## 3. Core specifications (version-stable across v0.1–v0.4)

These specifications are the Ganglion core. They apply to all versions unless explicitly noted.

### 3.1 Peer identity

Every Ganglion participant — operator, robot, relay — has a libp2p peer identity derived from an Ed25519 keypair. The peer ID is the canonical identifier in logs, capability policies, and audit records. Human-readable names (fleet names, robot names, operator names) are bindings over peer IDs maintained in a local registry; they are not authoritative.

**Rationale:** human-readable names change; peer IDs don't. Audit trails bound to peer IDs survive renaming, migration, and organizational reshuffling.

### 3.2 Roles

Three first-class roles:

- **Robot Agent.** The native Ganglion process running on a deployed robot. Dials out to one or more relays; hosts the WASM runtime and Layer 3 brokers.
- **Operator.** A human or automated system acting on a fleet. Interacts with robots via the `gang` CLI or operator libraries.
- **Relay.** A libp2p circuit relay that enables operators and robots to establish connections when neither can accept inbound.

A single process may act as multiple roles (e.g., an operator laptop can run a relay for testing). Production deployments separate them.

### 3.3 Connection model

Robot agents are outbound-initiated. On startup:

1. Robot agent generates or loads its keypair
2. Connects outbound to one or more configured relays over the best available transport
3. Registers its peer ID with the relay and remains available for reservation
4. Announces its existence to authorized operators via a signed presence message

Operators connect to the same relay(s) and request a circuit to the robot. Where NAT configurations permit, the relay facilitates a direct connection upgrade (DCUtR) so subsequent traffic flows peer-to-peer without transiting the relay. Where NAT configurations do not permit, traffic stays on the relay.

**Rationale:** this gives us the operational simplicity of a broker model with the efficiency of peer-to-peer where possible. No operator or customer-site network configuration is required beyond standard outbound HTTPS.

### 3.4 Stream protocol

All application-level traffic flows over libp2p streams multiplexed on the connection. Ganglion defines a small set of stream protocols:

- `/ganglion/control/1.0` — control messages (capability deployment, capability invocation, presence, configuration)
- `/ganglion/tool/1.0` — bidirectional stream between operator and an invoked capability
- `/ganglion/bulk/1.0` — high-volume artifact transfer (log bundles, rosbags, diagnostic tarballs)

Each protocol uses length-prefixed framing with CBOR-encoded messages. CBOR chosen for schema-evolvability, compactness, and non-JSON-adjacent debuggability.

**Rationale:** separate protocols allow flow control, priority, and cancellation per purpose. Bulk transfers cannot block control messages. A stuck tool invocation cannot wedge the control plane.

### 3.5 Capability interface (WIT)

All WASM components interact with the host through a WIT-defined interface. The v0.1 interface exposes four capability groups:

- `ganglion:ros/interface` — read-only and read-write access to ROS 2 topics, services, and parameters, gated by topic/service/parameter patterns
- `ganglion:logs/stream` — read access to system logs, journald, and ROS log files, gated by log source patterns
- `ganglion:fs/bounded` — bounded filesystem access, gated by path patterns with explicit read/write/execute flags
- `ganglion:diagnostics/collect` — structured diagnostic collection primitives (system info, process list, network state)

A component declares which capabilities it needs in its signed manifest. The host's policy engine evaluates the declaration against active policy at load time and refuses components whose declarations exceed what policy permits.

**Rationale:** capability declaration happens once, at load time, in a form operators and auditors can read. Runtime surprises are impossible because unrequested capabilities are literally unavailable.

#### 3.5.1 Language-neutrality of the capability interface

WIT interfaces are language-neutral by design. Capabilities may be authored in any language with a WebAssembly component toolchain, including Rust, C++, Go, TinyGo, Python (via componentize-py), and JavaScript/TypeScript (via ComponentizeJS). A capability's language is invisible to the host — the runtime loads a signed `.wasm` component and invokes it through the WIT-defined entrypoint regardless of what language authored it.

This matters because ROS 2 development is not a Rust-first community. The canonical client libraries are C++ (`rclcpp`) and Python (`rclpy`), with Rust (`rclrs`) growing but still a minority. Ganglion's capability authoring experience is aligned with that reality: **capability authors work in the language their team already uses**, not the language Ganglion's host happens to be implemented in.

Language support tiers, as of this specification version:

- **Tier 1 (production-ready):** Rust, C, C++, Go, TinyGo. Mature toolchains, idiomatic WIT bindings, straightforward component builds.
- **Tier 2 (working, rough edges):** Python (componentize-py), JavaScript/TypeScript (ComponentizeJS). Functional toolchains; larger binaries because interpreters ship inside the component; slightly non-native type ergonomics at the WIT boundary; cold-start latency of ~500ms vs. ~10ms for Tier 1.
- **Tier 3 (experimental but viable):** C#/.NET, Java/JVM (via Kotlin/Native or TeaVM), Ruby. Toolchains exist and are improving.
- **Tier 4 (not yet supported):** Languages without WASI component support — Swift (partial), R, MATLAB, most DSLs.

The Tier distinctions are informational, not prescriptive. A Python-authored capability is a first-class capability; it runs with the same capability enforcement, the same signing requirements, and the same audit behavior as a Rust-authored one. The only operational distinctions are binary size and cold-start time, which matter for high-frequency capabilities and are irrelevant for the operator-initiated workflows Ganglion primarily targets.

The v0.1 reference capability (`gang-capability-diagnostics`) is authored in Rust for implementation expedience. The v0.4 standard library (§7) deliberately includes capabilities authored in C++ and Python to demonstrate and validate the multi-language authoring pathway.

**Rationale:** a reviewer or adopter asking "do my C++ roboticists have to learn Rust to extend this system?" should be able to read this section and answer themselves: no. The host is Rust because rust-libp2p and Wasmtime are Rust; capability authoring is not.

### 3.6 Signing and manifest

Every WASM component ships with a manifest containing:

- Component name and version
- Declared capabilities
- Author peer ID
- Signature over the component bytes and manifest by the author's private key
- Optional: maximum memory, CPU budget, wall-clock deadline

The robot agent verifies signatures against its local trust store before loading. Unsigned components are rejected. Components signed by untrusted peers are rejected. Components that exceed policy-permitted capabilities are rejected.

### 3.7 Audit record

Every capability invocation produces an audit record:

- Invoking operator peer ID
- Component name, version, hash
- Declared capabilities used
- Wall-clock start/end
- Exit status
- Bytes in / bytes out on each host capability

Audit records are written to a local append-only log on the robot. Replication of audit records to an external store is out of scope for OSS Ganglion (it belongs in commercial layers).

**Rationale:** the robot produces evidence locally, always. Whether that evidence gets centralized is a deployment decision, not an architectural one.

### 3.8 Versioning and compatibility

Ganglion follows semantic versioning for protocol and interface definitions. The v0.x series will have breaking changes between minor versions; v1.0 marks the first stability commitment.

- **Stream protocols** version independently (`/ganglion/control/1.0` vs. `/ganglion/control/2.0`). Agents negotiate the highest mutually-supported version at connection time.
- **WIT interfaces** version independently per capability group. A component declaring `ganglion:ros/interface@1.0` runs on any host supporting 1.x of that interface.
- **The `gang` CLI** versions independently and must remain compatible with the two most recent minor Ganglion versions.

**Rationale:** field-deployed robots cannot upgrade in lockstep with operators. The protocol must tolerate version skew across the fleet.

---

## 4. v0.1 — Reference control plane (target: April 26, 2026)

v0.1 is the ROSCon 2026 submission artifact. It demonstrates that the architecture works, end to end, for one operator and one robot across a simulated hostile network.

### 4.1 Scope

**Included:**

- `gang-core` crate: libp2p host, stream protocol, capability policy engine, audit logger
- `gang-libp2p` crate: libp2p transport adapter (TCP + QUIC, circuit relay v2, DCUtR)
- `gang-ros` crate: ROS 2 integration — bridge from `/ganglion/control/1.0` to rosbridge WebSocket, plus structured log streaming from `/rosout`
- `gang-wasm-host` crate: WASM runtime (Wasmtime), WIT binding generation, capability enforcement
- `gang` CLI binary: operator tool for connecting to robots, listing capabilities, invoking capabilities, streaming results
- One reference WASM capability authored in Rust: `gang-capability-diagnostics` — collects system info, ROS node list, active topics, `/diagnostics` aggregate, last N seconds of rosout, returns a signed CBOR bundle
- Test harness: four Docker-compose scenarios emulating open warehouse, NAT'd office, enterprise DMZ, and mobile/CGNAT archetypes using `tc`/`netem` and `iptables`
- `docs/SPEC.md`: this document
- `docs/decision-flowchart.svg`: one-page architectural selection flowchart
- `docs/VALIDATION.md`: test harness results with measured RTT, connection-establishment time, throughput, and reconnect behavior for each archetype

**Excluded:**

- Multi-operator coordination
- Any transport beyond TCP and QUIC
- Browser-based operator UI
- Content-addressed artifact distribution
- Any WASM capability beyond diagnostics
- Reference capabilities in languages other than Rust (arriving in v0.4)
- Multi-tenancy
- SSO / enterprise auth
- HA relay deployment

### 4.2 Component-level design

**`gang-core`.** The central library. Hosts the libp2p node via a transport adapter trait. Owns the policy engine, the audit logger, and the WASM runtime handle. Exposes a Rust API consumed by `gang`, `gang-ros`, and operator tooling.

Key trait:

```rust
trait TransportAdapter: Send + Sync {
    async fn dial(&self, peer: PeerId) -> Result<Stream>;
    async fn listen(&self, protocol: ProtocolId, handler: StreamHandler) -> Result<()>;
    fn local_peer_id(&self) -> PeerId;
    fn capabilities(&self) -> TransportCapabilities;
}
```

`TransportCapabilities` describes what a transport supports (relay, hole-punching, direct dialing, encryption guarantees). `gang-core` selects strategies based on these capabilities without knowing transport internals.

**`gang-libp2p`.** Implements `TransportAdapter` using rust-libp2p. Configures identify, ping, kad (for peer routing through bootstrap nodes), circuit relay v2, and DCUtR behaviors. Supports TCP and QUIC as concrete transports; the adapter interface allows additional transports without modifying `gang-core`.

**`gang-ros`.** A native process on the robot that exposes ROS 2 resources to WASM capabilities via the Layer 3 broker interface. v0.1 implements the `ganglion:ros/interface` capability group by wrapping rclrs (ROS 2 Rust client). Topic subscriptions, service calls, and parameter operations flow through this broker, enforcing the pattern-based gating from the capability manifest.

**`gang-wasm-host`.** Hosts WASM components using Wasmtime. Generates Rust bindings for the WIT capability interface. Enforces memory limits, CPU budgets, and wall-clock deadlines from the component manifest. Connects component capability calls to the Layer 3 brokers.

**`gang` CLI.** The operator's front door. Commands for v0.1:

```
gang connect <robot>                    # establish session with a robot
gang list                               # list reachable robots in the fleet
gang caps <robot>                       # list capabilities installed on the robot
gang deploy <robot> <wasm-path>         # install a signed capability
gang run <robot> <cap-name> [args]      # invoke an installed capability
gang logs <robot> [--follow]            # stream robot logs
gang test-archetype <archetype>         # run local test harness scenario
```

**`gang-capability-diagnostics`.** A minimal WASM component authored in Rust, compiled to `wasm32-wasi` and produced as a component via `wasm-tools`. Declares `ganglion:ros/interface@1.0` (read-only), `ganglion:logs/stream@1.0`, and `ganglion:diagnostics/collect@1.0`. On invocation, collects system and ROS diagnostics, assembles a CBOR bundle, signs with the operator's key, and streams back to the operator. Serves as the worked example for authors of additional capabilities.

### 4.3 Bootstrap relay

Tafy Labs operates a single public bootstrap relay at `relay.gang.tafy.dev` for OSS users. The relay is single-tenant in the sense that it serves the global public namespace; it has no concept of customer organizations or policy-bounded fleets. Users who need multi-tenancy, SLA, or geographic distribution run their own relay.

Relay software is identical to any Ganglion node in relay mode — no separate codebase. The public deployment uses:

- Single VPS, IPv4+IPv6, sufficient egress for modest beta usage
- `gang-libp2p` in relay-only mode
- Observability via Prometheus metrics endpoint (public)
- No user registration, no rate limits beyond libp2p defaults

Expected cost: $20–50/month at beta traffic levels. Tafy Labs absorbs this as OSS contribution.

### 4.4 Test harness

Four scenarios, each a Docker-compose file plus shell glue. Each scenario spins up:

- A simulated customer-site network namespace with specified NAT, firewall, and latency characteristics
- A robot agent inside that namespace
- An operator agent outside
- A relay on a third network segment

The harness then measures:

- Time to first successful connection
- Steady-state RTT for control messages
- Throughput for bulk transfers
- Reconnection behavior under transient failures
- Direct-connection upgrade success rate (where applicable)

Results are captured in `VALIDATION.md` with concrete numbers. This file is part of the submission artifact.

### 4.5 What success looks like for v0.1

A ROSCon reviewer (or any reader) can:

- Clone the repo, run `cargo install --path .`, and have `gang` on their PATH in under 5 minutes
- Run `gang test-archetype enterprise-dmz` and see a simulated robot become reachable from their laptop in under 60 seconds
- Deploy the diagnostics capability to that simulated robot and receive a signed bundle of actual diagnostic data
- Read `SPEC.md` and understand what v0.2+ will add without having to read code

---

## 5. v0.2 — Transport breadth (target: mid-May 2026)

v0.2 expands the transport portfolio to cover the full realistic range of customer network environments.

### 5.1 Scope additions

**`gang-libp2p` additions:**

- WebTransport support for operator environments where only HTTPS/443 egress is permitted
- WebRTC transport for browser-based operator UI (lays groundwork for v0.2.x UI work)
- Noise handshake tuning for high-latency mobile/satellite environments
- Happy-eyeballs-style transport selection: attempt QUIC, TCP, and WebTransport in parallel; use whichever establishes first

**`gang` CLI additions:**

- `gang diagnose <robot>` — runs the network-archetype-detector capability (also new in v0.2) and reports the active transport path, relay hops, and observed network constraints
- `gang transport-stats <robot>` — shows per-transport statistics for an active connection

**New WASM capability:**

- `gang-capability-network-archetype` — probes the local network from inside the robot, identifies which archetype (open, NAT'd, DMZ, regulated, mobile) the robot is deployed into, and reports back. Useful for operators who don't know what they've walked into.

### 5.2 Component-level design

**WebTransport adapter.** Uses `quinn` (QUIC implementation) with WebTransport framing. For operator environments restricted to HTTPS-only outbound, WebTransport provides datagram and stream semantics over a genuine HTTPS connection that cannot be distinguished from normal web traffic at the L7 boundary.

**WebRTC transport.** Uses the rust-libp2p WebRTC transport. Enables eventual browser-based operator UIs without requiring a separate transport. In v0.2, exposed through the Rust API; actual browser UI is a v0.2.x or v0.3 item.

**Happy-eyeballs selection.** When connecting to a robot, `gang-libp2p` initiates parallel connection attempts across available transports and uses the first successful handshake. Remaining attempts are cancelled. Selection preferences are configurable; default prioritizes QUIC, then WebTransport, then TCP.

**Network-archetype detection.** The new capability runs standard network probes (outbound connectivity tests, MTU discovery, multicast reachability checks, DNS behavior characterization, STUN queries) and classifies the result. The classification output conforms to a machine-readable schema so operator tooling can react programmatically (e.g., auto-selecting configuration profiles based on archetype).

### 5.3 Breaking changes from v0.1

- `TransportAdapter` trait gains a `dial_parallel` method for happy-eyeballs; existing implementations compile but ignore parallelism
- `gang connect` gains `--prefer-transport` flag; default behavior changes from "try TCP then QUIC" to happy-eyeballs

### 5.4 What success looks like for v0.2

An operator in a corporate environment where only HTTPS/443 is permitted egress can still reach a robot behind a customer's NAT. The connection uses WebTransport on the operator side and QUIC between relay and robot. A browser-based operator UI is feasible to build on top.

---

## 6. v0.3 — Content-addressed forensic artifact distribution (target: early June 2026)

v0.3 makes libp2p's second great feature — content-addressed distribution — available to Ganglion users. This is the single feature that turns "libp2p is a fine transport choice" into "libp2p is obviously correct for this problem."

### 6.1 Scope additions

**New crate: `gang-artifacts`.**

- Content-addressed storage API with CID (Content Identifier) v1 + Blake3 hashing
- Chunking, dedup, and resumable transfer via bitswap-equivalent block exchange
- Local content store with configurable size caps and eviction policies
- Integration with `gang-capability-diagnostics` and future capabilities that produce bulk output

**`gang-ros` additions:**

- `gang-capability-rosbag-slice` — a new capability that captures a time-bounded slice of a running rosbag, stores it content-addressed, and returns the CID to the operator

**`gang` CLI additions:**

- `gang fetch <cid>` — retrieve a content-addressed artifact from any reachable peer that has it
- `gang push <path>` — publish a local artifact to the local content store and announce its CID
- `gang artifacts list` — list locally-stored artifacts with their CIDs, sizes, and origins

### 6.2 Why content-addressed distribution matters for robotics

Three properties that conventional file transfer doesn't give you:

**Deduplication across the fleet.** A single binary (firmware update, WASM capability, reference rosbag) distributed to 50 robots transfers once per network boundary, not 50 times. Critical when bandwidth between relay and robots is constrained.

**Resumability and caching.** A rosbag transfer interrupted at 80% resumes from the last block. A rosbag fetched by two operators from the same robot transfers once; the second operator fetches from the first operator's cache if available.

**Verifiability.** The CID is a hash. If two peers say they have the same artifact and their CIDs match, they have the same artifact, guaranteed. No separate signature verification for integrity (signatures remain necessary for authenticity).

### 6.3 Component-level design

**Storage layer.** Uses the `beetswap` crate (Rust bitswap implementation) for block exchange; blocks addressed by CIDv1 with Blake3. Local store backed by a content-addressed filesystem layout with SQLite index for metadata. Size cap and LRU eviction configurable per deployment.

**Integration with capabilities.** WASM capabilities that produce bulk output gain access to a new `ganglion:artifacts/publish@1.0` capability group. Invoking it on a byte stream produces a CID; the CID is returned to the operator as part of the capability's structured result. The operator then fetches the artifact via `gang fetch`, potentially from a closer peer than the robot itself.

**Rosbag slicing.** The new capability integrates with `rosbag2` storage; specifies start/stop times and topic filters; produces a proper rosbag2 bundle that can be replayed locally. Non-trivial but well-scoped.

### 6.4 Breaking changes from v0.2

- Diagnostics capability result format gains an optional `artifacts` field listing CIDs of large attachments (previously inlined in the CBOR bundle)
- Manifest schema v1.1: adds optional `artifact-capabilities` declarations

### 6.5 What success looks like for v0.3

An operator investigating a production incident runs:

```
gang run robot-42 rosbag-slice --start=-60s --end=now --topics=/odom,/scan,/cmd_vel
```

The robot captures the slice, publishes it content-addressed, and returns the CID. The operator fetches it locally in under 30 seconds. A second operator working on the same incident fetches from the first operator's cache in under 2 seconds without re-contacting the robot.

---

## 7. v0.4 — Capability standard library (target: early July 2026)

v0.4 shifts Ganglion from "a platform with one example capability" to "a platform with a standard library of capabilities operators actually need." This version also **validates the multi-language capability authoring pathway** by shipping reference capabilities in Rust, C++, and Python, and establishes the community contribution pathway for third-party capabilities in any Tier 1 or Tier 2 language.

### 7.1 Scope additions

**Expanded capability interface:**

- `ganglion:process/spawn@1.0` — bounded subprocess invocation with captured stdio and resource limits (for wrapping existing CLI tools as capabilities)
- `ganglion:network/probe@1.0` — structured network probing primitives (used by the archetype detector and new diagnostics)
- `ganglion:metrics/emit@1.0` — structured metric emission from capabilities back to the operator

**Standard capability library** (all signed by Tafy Labs, shipped via the Ganglion registry). **Authored in deliberately varied languages to demonstrate and validate multi-language authoring:** 

- `gang-capability-log-normalize` — **authored in Python** via componentize-py. Converts varied log formats (systemd, ROS, custom) into a structured normalized format for fleet-wide analysis. Text processing is Python's home turf; this capability exercises the componentize-py toolchain on a realistic workload.
- `gang-capability-topic-echo` — **authored in C++** via the wasi-sdk and `wit-bindgen` for C++. Subscribes to specified ROS topics and streams serialized messages to the operator with optional decimation. C++ is the ROS 2 community's primary language; this capability demonstrates native-language parity.
- `gang-capability-param-inspect` — authored in Rust. Snapshots current parameter server state, optionally diffs against a reference.
- `gang-capability-diagnostic-bundle` — authored in Rust. v2 of diagnostics, adds journald excerpts, dmesg, systemd unit status, ROS node graph, network state.
- `gang-capability-network-archetype` — authored in Rust. v2 of archetype detector, adds recommendation output.
- `gang-capability-canary-probe` — **authored in Go via TinyGo**. Quick health check primitive for fleet-scale "is this robot responsive" polling. A fourth language demonstrates the authoring pathway is genuinely open.

**Community pathway:**

- `docs/CAPABILITY_AUTHOR_GUIDE.md` — how to build, sign, and distribute a capability, with language-specific subsections for Rust, C++, Python, and Go/TinyGo
- `gang capability scaffold <name> --language=<rust|cpp|python|go>` — CLI command to generate a capability project skeleton for the chosen language
- Reference capability templates at `tafylabs/gang-capability-template-{rust,cpp,python,go}`

**Capability registry:**

- `registry.ganglion.tafy.dev` — a content-addressed registry of published capabilities with metadata (author, version, declared capabilities, signatures, authoring language)
- `gang registry search <query>` — discover capabilities
- `gang registry install <name>` — fetch and verify a capability from the registry

### 7.2 Component-level design

**Process broker.** Mediates subprocess invocation from WASM components. Enforces CPU, memory, wall-clock, and filesystem bounds on the subprocess. Captures stdio and streams it back to the capability. This is deliberately a narrow interface — components cannot invoke arbitrary processes, only those declared in the capability manifest against host-defined allowlists.

**Registry protocol.** Registry entries are capability manifests with signatures; fetching an entry returns the manifest and a CID for the WASM component itself (retrieved via the v0.3 content-addressed layer). The registry is a libp2p pubsub topic plus a content-addressed document graph — no central database.

**Breaking changes.** Capability manifest schema v2.0: formalizes capability group versioning, adds registry metadata fields including authoring language. Manifest v1.x components continue to load with a deprecation warning.

### 7.3 Why the boundary stays where it is

v0.4 is the version where someone might reasonably ask "is this a product yet?" The answer remains no, and the reasons remain consistent:

- **Multi-tenancy still absent.** The registry has one global namespace. Customer-scoped registries with isolated trust stores are commercial.
- **No SSO.** Capability signing uses peer keys; no SAML, no SCIM, no identity federation.
- **Audit records remain local.** A commercial deployment needs signed, append-only, centrally-aggregated audit logs with retention guarantees. Ganglion produces the records; Ganglion does not centralize them.
- **No SLA, no support contract.** Best-effort OSS; breakage is filed as issues, not incidents.
- **Capability policy engine is local-only.** Commercial deployments need fleet-wide policy with governed rollout, which is not in scope.

### 7.4 What success looks like for v0.4

An operator at a small robotics company installs `gang`, adds their fleet's robots, and uses the standard library to:

- Bundle diagnostics from any robot in 10 seconds
- Slice a rosbag from any robot in 30 seconds
- Stream normalized logs from all robots to a local analysis tool
- Scaffold, sign, and deploy a custom capability for their specific robot platform in under an hour in the language their team already knows

A third developer publishes a platform-specific capability to the registry. Other users install it via `gang registry install`. The community pathway is operational across all four reference languages.

---

## 8. Post-v1.0 and commercial-only items

The following items are intentionally out of scope for Ganglion OSS. Commercial products built on Ganglion address them:

**Multi-tenant isolation.** Customer-org-scoped fleets with isolated trust stores, registries, and audit streams. Enforced at the relay and at every robot agent.

**Enterprise identity.** SAML SSO, SCIM provisioning, role-based access control per customer organization. Operator identity federated against customer or vendor IdPs.

**Compliance artifacts.** HIPAA audit trails, SOC 2 evidence collection, change-management webhooks, signed immutable audit logs with retention guarantees. The shape of "what a compliance officer needs" varies enough per regulated industry that this is deliberately not in OSS.

**High-availability relay infrastructure.** Geo-distributed relay mesh with automatic failover, health monitoring, DDoS protection, and capacity scaling. Single-relay deployments work for small fleets; production deployments across regulated industries need more.

**Governed capability policy.** Fleet-wide policy engines that gate which capabilities may run where, with approval workflows, staged rollouts, and revocation. OSS Ganglion gives each robot a local policy; production needs coordinated policy.

**On-call integration and support.** PagerDuty integration, 24/7 escalation paths, guaranteed response SLAs, and the institutional knowledge required to navigate customer IT departments in regulated industries. This is service work, not code.

**Deployment playbooks for specific archetypes.** The five-archetype framework is OSS; the detailed per-archetype deployment runbooks — complete with customer IT conversation scripts, approval workflow templates, and archetype-specific configuration profiles — are commercial consulting IP.

The line is drawn where the work shifts from "correctness" (which OSS can demonstrate) to "durability, governance, and accountability" (which require a named entity taking responsibility).

---

## 9. Security model

### 9.1 Trust assumptions

- The operator's private key is held securely by the operator and is the root of operator authority
- The robot agent's private key is held by the robot and rooted in a local TPM or filesystem with appropriate permissions
- The relay is untrusted for confidentiality (it sees only encrypted traffic) but trusted for liveness (an adversarial relay can deny service)
- The customer's network is fully untrusted — any observer may see metadata (connection timing, sizes, destination IP); no observer can decrypt payload
- WASM components are untrusted beyond their signature; a signed component is trusted to the level of its declared capabilities and no further, regardless of the language it was authored in

### 9.2 Threat model

Threats Ganglion addresses:

- Passive network observation by customer IT or other observers
- Active MITM by on-path adversaries (addressed by libp2p Noise handshakes with peer ID verification)
- Capability overreach — a buggy or malicious WASM component attempting operations beyond its declared capabilities (addressed by the capability enforcement model, which does not depend on authoring language)
- Supply-chain attacks on capabilities (addressed by signature verification and the registry's content-addressed distribution)

Threats Ganglion does not address (these are out of scope or belong to other layers):

- Compromise of the operator's private key — if the operator is compromised, they are the attacker
- Compromise of the robot's native OS — once an attacker has root on the robot, Ganglion cannot protect against them
- Side-channel attacks against WASM sandboxes — Wasmtime provides the sandbox; Ganglion inherits its guarantees and limits
- Denial of service by the relay — addressed operationally (run your own relay or use multiple relays), not architecturally

### 9.3 Key rotation

Key rotation is supported via libp2p's peer ID mechanism — a new keypair generates a new peer ID, and the binding between human-readable names and peer IDs is updated in local registries. Rotation is a manual operation in OSS Ganglion (run a new keypair, update the robot agent, update the operator's known-peers list). Automated, policy-driven rotation is commercial.

---

## 10. Operational guidance

### 10.1 Recommended deployment (v0.4 era)

For a robotics company deploying a small fleet (1–20 robots) into a handful of customer sites:

1. Deploy `gang-ros` as a systemd service on each robot, configured to dial the Tafy Labs public relay (OSS) or a self-hosted relay
2. Operators use the `gang` CLI with their personal keypairs; public keys are added to the robots' trust stores at provisioning time
3. Customer IT requires nothing beyond standard outbound HTTPS/443
4. Capabilities are signed by the company's release engineer and deployed via CI/CD to the registry; operators install from the registry. Capability authors on the robotics team work in Rust, C++, Python, or Go per team preference; the host runs all of them identically.
5. Audit records accumulate on each robot; for incident investigation, operators use the diagnostics capability to extract them

### 10.2 When you've outgrown OSS Ganglion

Signs a deployment has outgrown what OSS can safely cover:

- More than one customer organization using the same operator pool, where customers must not see each other's robots
- Regulatory requirements for centralized, immutable audit logs
- SLAs that require 24/7 response guarantees
- Fleet size exceeding what a single-operator manual workflow can manage (roughly 50+ robots)
- Enterprise security teams requiring SSO and identity federation before approving deployment

At that point: either engage commercial services built on Ganglion, or contribute the missing pieces upstream (some of them are legitimately community-worthy).

---

## 11. Open questions

Items deliberately left open for community input during the v0.x cycle:

- Whether to provide a first-class browser-based operator UI in the OSS repo or treat UI as an ecosystem concern
- Whether the capability registry should be a single global namespace or structured by organization from the start
- What the v1.0 stability commitments should cover (stream protocols vs. WIT interfaces vs. CLI surface)
- Whether to pursue additional language support beyond the v0.4 four-language reference set — specifically C#/.NET and Java/JVM — based on community demand signals

These questions will be resolved through RFC-style proposals in the repo's `docs/rfc/` directory between v0.1 and v1.0.

---

## 12. Acknowledgments

Ganglion's design draws from:

- libp2p and the IPFS project for connectivity primitives
- The Bytecode Alliance and WASI project for the WASM component model and the language-agnostic capability authoring model
- Aktoh Cyber's Synapse platform for the cross-industry architectural pattern this work ports to robotics
- Clearpath Robotics' ROSCon 2024 networking workshop for clarifying where the community's existing understanding stops and where the next layer of problems begins
- The ROS 2 Industrial Maintenance Working Group for articulating the 5-to-500-robot scaling problem that Ganglion sits inside
