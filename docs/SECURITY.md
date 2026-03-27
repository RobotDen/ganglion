# Ganglion Security Model

This document describes Ganglion's threat model, security mechanisms, and trust boundaries.

## Threat model

Ganglion operates in a specific threat environment: robots deployed on customer networks where the network itself is untrusted and potentially hostile. The operator needs to reach robots, deploy diagnostic tools, and retrieve data — all without inbound connectivity and without trusting the network path.

### Actors

| Actor | Trust level | Description |
|-------|-------------|-------------|
| Operator | Trusted (authenticated) | Human or automated system with a valid Ed25519 keypair listed in the trust store |
| Robot agent | Trusted (local) | Native process on the robot that enforces all security policies |
| Relay server | Untrusted transport | Routes encrypted traffic; cannot read or modify payloads |
| Network path | Hostile | May drop, reorder, inject, or inspect packets |
| WASM component | Untrusted code | Runs in a sandbox with no ambient authority |

### What we defend against

1. **Unauthorized capability deployment** — Only operators whose public key is in the trust store can deploy components. Manifests are signed and verified.
2. **Capability privilege escalation** — Components declare what they need; the policy engine denies anything not explicitly permitted. Default-deny.
3. **Sandbox escape** — WASM components run in Wasmtime with fuel metering and wall-clock deadlines. They cannot access the host except through broker-mediated capability interfaces.
4. **Network eavesdropping** — All peer-to-peer traffic uses Noise protocol encryption (libp2p). The relay sees only encrypted bytes.
5. **Filesystem traversal** — The filesystem broker enforces a symlink jail. Paths that resolve outside the jail root are rejected.
6. **Arbitrary command execution** — The process broker enforces a command allowlist with glob patterns. Commands not on the list are rejected.
7. **Resource exhaustion** — Fuel metering limits CPU consumption. Epoch-based deadlines limit wall-clock time. Process broker enforces wall-clock timeouts on subprocesses.
8. **Audit evasion** — Every invocation produces an append-only audit record with operator identity, component hash, capabilities used, and I/O statistics.

### What we do NOT defend against

- **Compromised robot agent process** — If the agent binary itself is replaced, all bets are off. The agent is the trust root on the robot.
- **Physical access attacks** — An attacker with physical access to the robot can extract keys, modify the agent, or replace the OS.
- **Relay denial of service** — The relay is a shared resource. A DDoS against the relay prevents connectivity but cannot compromise data.
- **Supply chain attacks on WASM toolchains** — Ganglion verifies the signature on the final binary, but does not audit the compiler or build pipeline that produced it.
- **Side-channel attacks on WASM runtime** — Wasmtime provides logical isolation, not constant-time guarantees.

## Identity

### Ed25519 keypairs

Every peer (robot agent, operator, relay) has an Ed25519 keypair:

- **Private key**: 32 bytes, stored at `~/.gang/identity.key` (file permissions should be `0600`)
- **Public key**: Derived from the private key
- **Peer ID**: `12D3-` prefix + hex-encoded first 16 bytes of Blake3 hash of the public key

Key generation uses `OsRng` via the `rand` crate (OS-level CSPRNG).

```bash
gang identity generate        # Creates ~/.gang/identity.key
gang identity show            # Displays peer ID and public key hex
```

### Trust store

The trust store (`~/.gang/trusted_peers.json`) lists public keys that are authorized to deploy capabilities. The robot agent checks the trust store during manifest verification.

Adding a peer to the trust store is an explicit operator action — there is no automatic trust establishment.

## Manifest signing

Every WASM component must have a signed manifest before deployment:

```bash
gang sign my-capability.wasm --name my-capability --version 0.1.0
# Produces: my-capability.manifest.cbor
```

The manifest contains:

| Field | Purpose |
|-------|---------|
| `name` | Human-readable component name |
| `version` | Semantic version |
| `author_peer_id` | Deployer's peer ID |
| `component_hash` | Blake3 hash of the `.wasm` binary |
| `declared_capabilities` | List of capability groups the component needs |
| `signature` | Ed25519 signature over the serialized manifest |
| `schema_version` | Manifest schema version (v2.0) |
| `language` | Source language (Rust, C++, Python, Go, Other) |
| `description` | Short description |
| `tags` | Searchable tags |
| `min_ganglion_version` | Minimum compatible Ganglion version |

### Verification at deploy time

When an operator deploys a component to a robot:

1. Robot agent deserializes the manifest (CBOR)
2. Verifies the Ed25519 signature against the author's public key
3. Checks that the author's public key is in the trust store
4. Computes Blake3 hash of the `.wasm` binary and compares against manifest
5. Evaluates every declared capability against the policy engine
6. Only if all checks pass: stores the component and manifest to disk

## Policy engine

The policy engine is default-deny and is defined in TOML:

```toml
# Allow diagnostics collection
[[capability_rules]]
group = "ganglion:diagnostics/collect"
allowed_patterns = ["**"]

# Allow ROS interface — read-only, diagnostics topics only
[[capability_rules]]
group = "ganglion:ros/interface"
allowed_patterns = ["/diagnostics/**", "/rosout"]
max_access = "read_only"

# Allow filesystem access — only under /tmp/gang
[[capability_rules]]
group = "ganglion:fs/bounded"
allowed_patterns = ["/tmp/gang/**"]

# Allow process spawn — only echo and cat
[[capability_rules]]
group = "ganglion:process/spawn"
allowed_patterns = ["echo", "cat"]

# Allow any peer to deploy
[[peer_rules]]
peer_id = "*"
can_deploy = true
```

### Policy evaluation

Policy is evaluated at two points:

1. **Deploy time** — before the component is stored on disk. If policy rejects the declared capabilities, the deployment fails and nothing is written.
2. **Invoke time** — before the component is loaded into the WASM runtime. This catches policy changes between deploy and invoke.

### Pattern matching

Capability patterns use glob matching (via the `glob_match` crate):

- `**` matches everything
- `*` matches any single path segment
- `/diagnostics/**` matches `/diagnostics/cpu`, `/diagnostics/memory/heap`, etc.

## WASM sandbox

### Resource limits

- **Fuel metering**: Each component invocation gets a configurable fuel budget. One WASM instruction consumes one unit of fuel. When fuel is exhausted, execution traps.
- **Epoch-based deadlines**: The runtime checks wall-clock time at epoch boundaries. Components that exceed their wall-clock deadline are interrupted.
- **Memory limits**: Wasmtime's built-in memory limits prevent unbounded allocation.

### Capability enforcement

When a component is loaded, the runtime wires only the declared capabilities to the host. A component that declares `ganglion:diagnostics/collect` cannot call `ganglion:fs/bounded` functions — those imports are simply not linked.

### No ambient authority

WASM components have:

- No filesystem access (except through `fs/bounded` broker)
- No network access (except through `network/probe` broker)
- No process execution (except through `process/spawn` broker)
- No environment variables
- No access to stdin/stdout (stdio is captured, not inherited)
- No access to the host clock (except through metered host calls)

## Broker-level security

### Filesystem broker

- **Symlink jail**: All paths are resolved (canonicalized) and checked against the jail root. If the resolved path is outside the jail, the request is rejected.
- **Pattern matching**: Only paths matching the declared `FsAccessPattern` patterns are accessible.
- **Permission flags**: Each pattern has explicit `read`, `write`, `execute` flags.

### Process broker

- **Command allowlist**: The `allowed_commands` field on `ProcessSpawn` capabilities lists which commands can run, with glob support.
- **Wall-clock timeout**: Each subprocess has a configurable timeout. The broker enforces the minimum of the per-request timeout and the broker's maximum timeout.
- **Captured stdio**: Subprocess stdout/stderr are piped and returned as data. stdin is `/dev/null`. The subprocess never inherits the agent's terminal.

### Network probe broker

- Provides structured probing primitives (ping, DNS, port check, traceroute) rather than raw socket access.
- Userspace implementations: ping uses TCP connect to port 80, DNS uses standard library resolution, traceroute is a stub in userspace contexts.

## Audit logging

Every capability invocation produces an `AuditRecord`:

```rust
pub struct AuditRecord {
    pub operator_peer_id: PeerId,
    pub component_name: String,
    pub component_version: String,
    pub component_hash: String,
    pub capabilities_used: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub exit_status: ExitStatus,
    pub io_stats: Vec<CapabilityIoStats>,
}
```

Records are written as length-prefixed CBOR to an append-only log at `/var/lib/gang/audit.log`. The log rotates by size (configurable, default 10 MB).

The audit log is designed to be forensically useful: it records not just what ran, but who ran it, what capabilities were used, how much data was read/written, and whether execution succeeded or failed.

## Transport encryption

All peer-to-peer traffic is encrypted using the Noise protocol (XX handshake pattern) as implemented by libp2p. The relay server handles only encrypted ciphertext and cannot inspect or modify payloads.

QUIC connections provide their own TLS 1.3 encryption in addition to the Noise layer.

## Recommendations for deployment

1. **Restrict the trust store** — only add operator keys that need to deploy to specific robots.
2. **Use restrictive policies** — start with deny-all and add only the capabilities each component needs.
3. **Protect identity keys** — set file permissions to `0600` on `identity.key` files.
4. **Monitor audit logs** — forward audit logs to a central monitoring system for anomaly detection.
5. **Pin component hashes** — after verifying a component, record its Blake3 hash and verify on subsequent deploys.
6. **Run your own relay** — don't rely on third-party relays for production traffic.
