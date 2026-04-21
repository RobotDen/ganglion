# Validation results

Test results and validation status for Ganglion v0.6.0.

## Test environment

- **Host:** macOS / Linux
- **Ganglion version:** v0.6.0
- **Rust:** 1.85+
- **CI:** GitHub Actions (ubuntu-latest + macos-latest matrix)

## Unit test summary

188 tests across 9 crates, all passing:

| Crate | Tests | Coverage areas |
|-------|-------|---------------|
| gang-core | 46 | Identity (keypair gen, persist, sign/verify, registry), messages (CBOR framing, varint), manifests (sign, verify, tamper detection, hash verification, v2 fields, schema version), policy (default-deny, patterns, peer auth, TOML roundtrip, process/network/metrics groups, read-write required for param-set), audit (write/read, rotation), artifacts (CID determinism, dedup, chunking, LRU eviction, persist/reload), registry (publish, search, tags, versions, persist/reload, remove) |
| gang-ros | 84 | Diagnostics broker (system info, network, processes), filesystem broker (read/write/list/stat, pattern gating, symlink jail, write-to-new-file traversal denial, symlink parent jail, nonexistent parent denial), log stream broker (source enumeration, pattern filtering), ROS broker (check_access exact/glob/wildcard/denied/read-only/read-write/empty patterns/first-match, RosList filtering by allowed patterns, topic subscribe with/without ros2, service call with/without ros2, param get/set with/without ros2, access denied propagation, unsupported ops, capability group, input validation, timeouts), robot agent (deploy, invoke, trust verification, capability loading with trust store check), archetype detection (classify all 5 archetypes, display, recommendations), process broker (allowlist exact/glob/wildcard, spawn echo, denied command, handle request, unsupported op), network probe broker (DNS lookup, port check, traceroute, handle request, unsupported op), metrics broker (emit, batch, ring buffer eviction, drain, unsupported op) |
| gang-wasm-host | 18 | WIT interface parsing, component runtime setup, fuel metering, capability host (declared/undeclared/registered groups), WASM-to-broker import registration (all 8 interfaces), broker routing (declared/undeclared/missing), Val extraction (byte list, string list, option string) |
| gang-libp2p | 9 | Swarm config defaults, swarm build, relay server config, capability tracking, peer connection tracking, transport stats, peer ID determinism, protocol codec, request-response setup |
| gang-cli | 0 | Integration-tested via `gang demo`, `gang status`, and manual CLI exercises |
| gang-capability-diagnostics | 6 | Report construction, serialization roundtrip, format output sections, empty disk handling, optional fields |
| gang-capability-param-inspect | 8 | Snapshot construction, diff (added/removed/changed), empty snapshots, identical snapshots, format output, mixed types, nested paths |
| gang-capability-diagnostic-bundle | 7 | Bundle construction, system section, journal section, network section, ROS graph section, health analysis (memory/disk/CPU/systemd), severity ordering, report formatting |
| gang-capability-network-archetype | 10 | Probe construction, archetype classification (all 5 types), connectivity scoring (all pass, partial, all fail), recommendation generation, report formatting |

### Test breakdown by version

| Version | Tests added | Running total |
|---------|------------|---------------|
| v0.1.0 | 52 | 52 |
| v0.2.0 | 8 | 60 |
| v0.3.0 | 10 | 70 |
| v0.4.0 | 56 | 126 |
| v0.5.0 | 62 | 188 |
| v0.6.0 | 0 | 188 |

## CI pipeline

The GitHub Actions CI workflow runs on every push and pull request to `main`:

| Job | What it checks | Matrix |
|-----|---------------|--------|
| Check | `cargo check --all-targets` | ubuntu-latest |
| Format | `cargo fmt --check` | ubuntu-latest |
| Clippy | `cargo clippy --all-targets` with `RUSTFLAGS="-Dwarnings"` | ubuntu-latest |
| Test | `cargo test` | ubuntu-latest, macos-latest |
| Documentation | `cargo doc --no-deps` with `RUSTDOCFLAGS="-Dwarnings"` | ubuntu-latest |

All jobs must pass before merging.

## Pre-commit hooks

The pre-commit hook (`.githooks/pre-commit`) runs the same three checks locally:

1. `cargo fmt --check`
2. `cargo clippy --all-targets --quiet -- -D warnings`
3. `cargo test --quiet`

Set up with `./scripts/setup-hooks.sh`.

## Network archetype scenarios

Four Docker-compose scenarios simulate increasingly hostile network conditions.

### Scenario 1: Open warehouse

**Topology:** Flat L2 bridge network (172.20.0.0/24). Relay, robot, and operator all directly reachable.

| Metric | Expected | Notes |
|--------|----------|-------|
| Connection method | Direct TCP/QUIC | No relay needed |
| NAT traversal | N/A | No NAT in path |
| Firewall restrictions | None | All ports open |

### Scenario 2: NAT'd office

**Topology:** Two separate LANs (192.168.1.0/24, 192.168.2.0/24) behind independent NAT gateways, relay on shared internet (10.0.0.0/24).

| Metric | Expected | Notes |
|--------|----------|-------|
| Connection method | Relay, then DCUtR upgrade | Both sides behind NAT |
| NAT type | Endpoint-independent (MASQUERADE) | DCUtR should succeed |
| Inbound connections | Blocked | iptables DROP on NEW from external |

### Scenario 3: Enterprise DMZ

**Topology:** Robot on isolated VLAN (172.16.10.0/24) behind restrictive firewall. Only TCP 443 outbound permitted. 5ms netem delay simulating TLS inspection overhead.

| Metric | Expected | Notes |
|--------|----------|-------|
| Connection method | Relay only | QUIC (UDP) blocked |
| Allowed ports | TCP 443 outbound only | All others dropped |
| Additional latency | +5ms | TLS inspection simulation |
| DCUtR | Will fail | Firewall blocks UDP |

### Scenario 4: Mobile/CGNAT

**Topology:** Robot behind double NAT — inner (10.64.0.0/24 to 100.64.0.0/24) and outer CGNAT (100.64.0.0/24 to 10.2.0.0/24). Symmetric NAT on outer layer. Mobile network conditions simulated: 50ms +/- 30ms latency, 2% packet loss on inner; 20ms +/- 10ms, 0.5% loss on outer.

| Metric | Expected | Notes |
|--------|----------|-------|
| Connection method | Relay only | Symmetric NAT defeats DCUtR |
| NAT type | Symmetric (carrier) | Different mapping per destination |
| Base RTT overhead | ~70ms+ | Cumulative netem delays |
| Packet loss | ~2.5% cumulative | Inner + outer loss |
| DCUtR | Will fail | Symmetric NAT |

## Measured results

### Test harness infrastructure (2026-04-23)

**Status: Validated (infrastructure layer)**

The following components have been validated:

| Component | Status | Notes |
|-----------|--------|-------|
| `.dockerignore` | Added | Excludes `target/` (15 GB), `.git/`, docs from build context |
| `Dockerfile.base` | Fixed | Layer-cached manifest copy, strips `rust-toolchain.toml` (avoids wasm32-wasip2 target requirement in container), builds only `-p gang` |
| `gang` binary | Compiles | `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass (188/188 tests) |
| `gang relay` command | Validated | Starts libp2p relay server, generates identity, listens on TCP+QUIC, prints multiaddrs |
| `gang agent` command | Validated | Creates data dir, generates identity, runs until Ctrl+C |
| `gang test-archetype` CLI | Fixed | Checks for docker + docker compose, cleans up previous runs, runs connectivity checks, provides teardown instructions |
| Compose: open-warehouse | Ready | 3 services (relay, robot, operator) on flat 172.20.0.0/24 |
| Compose: nat-office | Ready | 5 services (relay, robot, operator, 2 NAT gateways), 3 networks |
| Compose: enterprise-dmz | Ready | 4 services (relay, robot, operator, firewall), TCP 443 only + netem |
| Compose: mobile-cgnat | Ready | 5 services (relay, robot, operator, inner NAT, outer CGNAT), netem jitter/loss |
| `run-tests.sh` | Created | Builds base image, runs each scenario, checks containers + processes + connectivity + network rules |

**Docker build validation** was attempted but Docker Desktop's containerd storage became corrupted during the initial build (I/O errors on `meta.db`). The daemon requires manual cleanup (`docker system prune` or container runtime reset) before builds can proceed. All Dockerfile and compose file changes have been validated structurally.

**Changes made to fix the test harness:**

1. **Relay nodes** now run `gang relay --port <port>` instead of `gang agent` -- the relay command starts the libp2p circuit relay v2 server, which is what NAT'd clients need to connect through.
2. **Operator nodes** now run `gang agent --data-dir /data` instead of `gang demo` -- the demo command exits immediately after running (it creates a throwaway local agent), which causes the container to stop.
3. **Enterprise DMZ relay** listens on port 443 (`gang relay --port 443`) to match the firewall's TCP 443 outbound-only rule.
4. **Dockerfile.base** copies only `Cargo.toml`, `Cargo.lock`, and `crates/` (not the full repo), removes `rust-toolchain.toml` to avoid the wasm32-wasip2 target requirement, and pre-creates `/data`.
5. **`.dockerignore`** added to exclude `target/` (15 GB), `.git/`, and other non-essential directories from the Docker build context.
6. **`gang test-archetype` CLI** now validates both `docker` and `docker compose` availability, tears down leftover containers before starting, runs archetype-specific connectivity checks, and cleans up on failure.
7. **`run-tests.sh`** created as comprehensive test runner with 6 checks per scenario: container state, relay process, robot process, relay startup logs, archetype connectivity, and archetype network rules.

### Pending validation (requires working Docker)

Once Docker is restored, run:

```bash
cd /path/to/ganglion
./test-harness/run-tests.sh
```

Expected results per scenario:
- **open-warehouse:** operator can ping robot at 172.20.0.20 (flat L2)
- **nat-office:** robot can reach gateway at 192.168.1.1, NAT DROP rules present
- **enterprise-dmz:** robot can reach firewall at 172.16.10.1, TCP 443 rule present
- **mobile-cgnat:** robot can reach inner NAT at 10.64.0.1, netem qdisc active

## Known limitations (v0.6)

- Docker test scenarios verify network topology and reachability; full end-to-end protocol flow testing requires live relay connectivity
- Regulated facility (air-gapped) archetype is not Docker-testable — requires physical sneakernet
- The `gang logs` and `gang list` commands require relay connectivity (not yet wired)
- Registry is local-only — distributed registry synchronization is planned for a future version
- The relay and agent nodes do not yet auto-discover each other within Docker scenarios; this requires passing the relay's multiaddr to the robot and operator containers (planned for the next harness iteration)
