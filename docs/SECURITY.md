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
8. **Audit evasion** — Every invocation produces an append-only, Blake3-hash-chained audit record with operator identity, component hash, capabilities used, and I/O statistics; `verify_chain()` detects reordering or deletion.

### What we do NOT defend against

- **Compromised robot agent process** — If the agent binary itself is replaced, all bets are off. The agent is the trust root on the robot.
- **Physical access attacks** — An attacker with physical access to the robot can extract keys, modify the agent, or replace the OS.
- **Relay denial of service** — The relay is a shared resource. A DDoS against the relay prevents connectivity but cannot compromise data.
- **Supply chain attacks on WASM toolchains** — Ganglion verifies the signature on the final binary, but does not audit the compiler or build pipeline that produced it.
- **Side-channel attacks on WASM runtime** — Wasmtime provides logical isolation, not constant-time guarantees.

## Identity

### Ed25519 keypairs

Every peer (robot agent, operator, relay) has an Ed25519 keypair:

- **Private key**: 32 bytes, stored at `~/.gang/identity.key`. Permissions are enforced: a key file with a mode looser than `0600` is repaired to `0600` (with a warning) before it is used, and new key files are created `0600`.
- **Public key**: Derived from the private key
- **Peer ID**: `12D3-` prefix + hex-encoded first 16 bytes of Blake3 hash of the public key

Key generation uses `OsRng` via the `rand` crate (OS-level CSPRNG).

### Unified peer-id derivation (SEC-03)

Both a peer's own id and the id derived for a *remote* peer by the libp2p
transport now use the same canonical scheme: the raw Ed25519 public key →
`PeerId::from_ed25519_bytes`. Previously the transport derived remote ids from
the libp2p multihash, which never matched the core derivation — so trust-store
`peer_rules` keyed on a remote id were silently ineffective. With derivation
unified, `peer_rules` are enforceable. This is a breaking change for any
recorded remote id (see [MIGRATION-v2.md](MIGRATION-v2.md)).

```bash
gang identity generate        # Creates ~/.gang/identity.key
gang identity show            # Displays peer ID and public key hex
```

### Trust store

The trust store (`~/.gang/trusted_peers.json`) lists public keys that are authorized to deploy capabilities. The robot agent checks the trust store during manifest verification.

Adding a peer to the trust store is an explicit operator action — there is no automatic trust establishment.

**Fail closed:** a malformed or unreadable trust store aborts agent startup. The agent never starts with an empty/permissive trust store as a fallback.

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

### Registry entries are authenticated against the manifest

The local capability registry accepts an entry only with an accompanying
signed manifest (SEC-15) and validates the entry **field-by-field** against it:
name, version, capabilities, and component CID must all match the
authenticated manifest contents. CLI overrides that contradict the manifest
(e.g. `gang registry publish --version`) are rejected, so registry metadata
cannot silently diverge from what was signed.

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

### Fail-closed loading

A malformed or unreadable policy file aborts agent startup. The agent refuses to
fall back to a permissive policy that would allow everything — a broken policy
is a hard error, not a silent downgrade. (An *absent* policy in local/dev mode
is a separate, explicit code path.)

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

- **Fuel metering**: Each component invocation gets a fuel budget derived from the manifest and bounded by a hard cap. One WASM instruction consumes one unit of fuel. When fuel is exhausted, execution traps.
- **Epoch-based deadlines**: The runtime checks wall-clock time at epoch boundaries. Components that exceed their wall-clock deadline are interrupted.
- **Memory limits**: A linear-memory ceiling derived from the manifest (bounded by a hard cap) is enforced via Wasmtime's `StoreLimits`; growing memory past the cap traps.
- **Integrity re-check**: Component bytes are re-hashed (Blake3) immediately before execution and compared against the manifest hash. A mismatch refuses execution. There is no silent fallback to direct broker invocation — a WASM execution failure is terminal.

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

- **Canonicalized symlink jail (TOCTOU-closed, SEC-10)**: Paths are canonicalized before use; for writes to new files, the *parent* directory is canonicalized. The canonicalized path is checked against the jail root, and callers operate on the returned canonical path — closing the time-of-check/time-of-use window where an attacker swaps in a symlink after the check. Paths that resolve outside the jail are rejected.
- **Final-component symlink rejection**: For writes to new files, a symlink at the final path component — including a *dangling* symlink, which parent canonicalization alone would not catch — is rejected, so a planted link cannot redirect the write outside the jail.
- **Pattern matching**: Only paths matching the declared `FsAccessPattern` patterns are accessible.
- **Permission flags**: Each pattern has explicit `read`, `write`, `execute` flags.

### Process broker

- **Absolute, allowlisted commands**: A command must be an absolute path and, after canonicalization (resolving symlinks and `..`), must match the allowlist. Relative commands and non-allowlisted paths are rejected.
- **Scrubbed environment**: The child environment is cleared (`env_clear`) before spawn; the broker sets the only `PATH` the child ever sees. The subprocess does not inherit the agent's environment.
- **Wall-clock timeout**: Each subprocess has a configurable timeout. The broker enforces the minimum of the per-request timeout and the broker's maximum timeout.
- **Captured stdio**: Subprocess stdout/stderr are piped and returned as data. stdin is `/dev/null`. The subprocess never inherits the agent's terminal.

### Network probe broker

- Provides structured probing primitives (ping, DNS, port check, traceroute) rather than raw socket access.
- **SSRF hardening**: Every probe target is checked against a configured host/CIDR allowlist. Sensitive ranges — IPv4 loopback (`127.0.0.0/8`), link-local/cloud-metadata (`169.254.0.0/16`, including `169.254.169.254`), IPv6 link-local (`fe80::/10`), IPv4-mapped-IPv6 addresses, and IPv6 ULA — are blocked *unconditionally*, regardless of the allowlist, to prevent SSRF and cloud-metadata exfiltration. An empty allowlist denies all targets.
- **DNS-rebinding resistance**: A hostname target is resolved once, the resulting addresses are canonicalized and vetted against the blocklist/allowlist, and probes then connect *only to those vetted addresses* — the hostname is never re-resolved for the connection, so a DNS answer that changes between check and use cannot redirect a probe to a blocked address.
- Userspace implementations: ping uses TCP connect, DNS uses standard library resolution, traceroute is a stub in userspace contexts.

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

Records are written as length-prefixed CBOR to an append-only log. Each record
is linked into a **Blake3 hash chain** — the stored hash for record *n* is
`blake3(prev_hash || seq || cbor(record))` — so any reordering, deletion, or
in-place edit breaks the chain. `AuditLog::verify_chain()` walks the log and
detects tampering. The log file is created with `0600` permissions and rotates
by size (configurable). On rotation, the new log file **carries the rotated
file's tip hash** as its starting `prev_hash`, so the chain spans rotated
files rather than restarting.

**Honest trust bounds:** the hash chain has no external anchor. An attacker who
can rewrite the whole log file (and its rotated predecessors) can recompute a
consistent chain — a *full rewrite is undetectable*. Likewise, *truncating
trailing records* is undetectable, because the shortened chain is still
internally consistent. What the chain detects is tampering *within* a log an
attacker cannot fully rewrite: reordering, deletion of interior records, and
in-place edits. For stronger guarantees, periodically export the tip hash to an
external system (see deployment recommendations).

The audit log is designed to be forensically useful: it records not just what ran, but who ran it, what capabilities were used, how much data was read/written, and whether execution succeeded or failed — and now whether the record sequence is intact.

## Replay protection (control plane)

Control requests carry a per-request nonce and a timestamp. The agent rejects
requests whose timestamp is outside the freshness window or whose nonce has
already been seen, defeating replay of a captured control message. Because the
nonce/timestamp are required, a pre-2.0 operator that omits them is rejected —
all agents and operators must run compatible versions (see
[MIGRATION-v2.md](MIGRATION-v2.md)).

The replay guard tracks at most **100,000 nonces** and **fails closed**: when
the guard is at capacity, new requests are rejected rather than accepted
untracked, so memory stays bounded without ever degrading into accepting
replays.

## Transport encryption

All peer-to-peer traffic is encrypted using the Noise protocol (XX handshake pattern) as implemented by libp2p. The relay server handles only encrypted ciphertext and cannot inspect or modify payloads.

QUIC connections provide their own TLS 1.3 encryption in addition to the Noise layer.

## Recommendations for deployment

1. **Restrict the trust store** — only add operator keys that need to deploy to specific robots.
2. **Use restrictive policies** — start with deny-all and add only the capabilities each component needs.
3. **Protect identity keys** — the agent enforces `0600` on `identity.key`, but keep the containing directory and backups equally restricted.
4. **Monitor audit logs** — forward audit logs to a central monitoring system for anomaly detection, and periodically record the chain's tip hash externally: the hash chain alone cannot detect a full rewrite or trailing truncation, but an externally anchored tip hash can.
5. **Pin component hashes** — after verifying a component, record its Blake3 hash and verify on subsequent deploys.
6. **Run your own relay** — don't rely on third-party relays for production traffic.
