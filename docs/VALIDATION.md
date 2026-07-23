# Validation results

Test results and validation status for Ganglion v2.0.0.

## Test environment

- **Host:** macOS / Linux
- **Ganglion version:** v2.0.0
- **Rust:** 1.88+ (workspace MSRV)
- **CI:** GitHub Actions (ubuntu-latest + macos-latest matrix)

## Unit test summary

337 tests across 13 crates, all passing, plus 1 ignored live-network archetype
test (requires internet access; run with `cargo test -- --ignored`):

| Crate | Tests | Coverage areas |
|-------|-------|---------------|
| gang-core | 87 | Identity (keypair gen, persist, sign/verify, registry), messages (CBOR framing, varint), manifests (sign, verify, tamper detection, hash verification, v2 fields, schema version), policy (default-deny, patterns, peer auth, TOML roundtrip, process/network/metrics groups, read-write required for param-set), audit (write/read, rotation, hash-chain verification, rotation tip-hash linkage), replay guard (nonce/timestamp freshness, capacity bound), artifacts (CID determinism, dedup, chunking, LRU eviction, persist/reload), registry (publish, search, tags, versions, persist/reload, remove, entry-vs-manifest validation) |
| gang-ros | 112 (+1 ignored) | Diagnostics broker (system info, network, processes), filesystem broker (read/write/list/stat, pattern gating, symlink jail, write-to-new-file traversal denial, symlink parent jail, final-component symlink rejection, nonexistent parent denial), log stream broker (source enumeration, pattern filtering), ROS broker (check_access exact/glob/wildcard/denied/read-only/read-write/empty patterns/first-match, RosList filtering by allowed patterns, topic subscribe with/without ros2, service call with/without ros2, param get/set with/without ros2, access denied propagation, unsupported ops, capability group, input validation, timeouts), robot agent (deploy, invoke, trust verification, capability loading with trust store check), archetype detection (classify all 5 archetypes, display, recommendations; 1 live-network test is `#[ignore]`d), process broker (allowlist exact/glob/wildcard, spawn echo, denied command, handle request, unsupported op), network probe broker (DNS lookup, port check, traceroute, address vetting, blocked-range rejection, handle request, unsupported op), metrics broker (emit, batch, ring buffer eviction, drain, unsupported op) |
| gang-wasm-host | 28 | WIT interface parsing, component runtime setup, fuel metering, component cache, capability host (declared/undeclared/registered groups), WASM-to-broker import registration (all 8 interfaces), broker routing (declared/undeclared/missing), Val extraction (byte list, string list, option string) |
| gang-libp2p | 27 | Swarm config defaults, swarm build, relay server config, capability tracking, peer connection tracking, transport stats, peer ID determinism, protocol codec, request-response setup, browser transport capability reporting (enabled/disabled), config defaults, multiaddr transport detection |
| gang-cli | 0 | Integration-tested via `gang demo`, `gang status`, and manual CLI exercises |
| gang-capability-diagnostics | 6 | Report construction, serialization roundtrip, format output sections, empty disk handling, optional fields |
| gang-capability-param-inspect | 8 | Snapshot construction, diff (added/removed/changed), empty snapshots, identical snapshots, format output, mixed types, nested paths |
| gang-capability-diagnostic-bundle | 7 | Bundle construction, system section, journal section, network section, ROS graph section, health analysis (memory/disk/CPU/systemd), severity ordering, report formatting |
| gang-capability-network-archetype | 10 | Probe construction, archetype classification (all 5 types), connectivity scoring (all pass, partial, all fail), recommendation generation, report formatting |
| gang-capability-log-normalize | 11 | ROS 2 line parsing, journald line parsing, syslog line parsing, plaintext fallback, empty line, batch normalization with mixed formats, severity keyword parsing, serialization roundtrip, report formatting, syslog severity extraction |
| gang-capability-topic-echo | 11 | Decimation (no reduction, every-second, with max, empty stream), sequence numbers, report aggregation, argument parsing (basic, defaults), serialization roundtrip, report formatting, byte tracking |
| gang-capability-canary-probe | 11 | Healthy robot, degraded memory, unhealthy disk, unreachable robot, recent reboot detection, missing data skip, oneliner formatting, report formatting, serialization roundtrip, custom thresholds, default thresholds |
| gang-capability-rosbag-slice | 19 | Time window parsing (relative times, "now", combined windows), topic filtering (exact, glob, mixed), bag format detection (sqlite3, mcap), record command building, slice config construction, argument parsing, report formatting, size formatting, serialization roundtrip |

### Test breakdown by version

Running totals are measured from the release tags (`cargo test` at each tag),
correcting earlier over-counted figures:

| Version | Tests added | Running total |
|---------|------------|---------------|
| v0.1.0 | 52 | 52 |
| v0.2.0 | 8 | 60 |
| v0.3.0 | 10 | 70 |
| v0.4.0 | 56 | 126 |
| v0.5.0 | 49 | 175 |
| v0.6.0 | 14 | 189 |
| v1.0.0 | 66 | 255 |
| v2.0.0 (security/quality audit + hardening) | 82 | 337 (+1 ignored) |

## CI pipeline

The GitHub Actions CI workflow runs on every push and pull request to `main`:

| Job | What it checks | Where it runs |
|-----|---------------|---------------|
| `fmt` | `cargo fmt --check` | ubuntu-latest |
| `clippy` | `cargo clippy --all-targets` with `RUSTFLAGS="-Dwarnings"` | ubuntu-latest |
| `test` | `cargo test` | ubuntu-latest, macos-latest |
| `doc` | `cargo doc --no-deps` with `RUSTDOCFLAGS="-Dwarnings"` | ubuntu-latest |
| `msrv` | Build on the minimum supported Rust version (1.88) | ubuntu-latest |
| `deny` | `cargo-deny` (licenses, advisories, bans, sources) | ubuntu-latest |
| `harness` | Docker test harness — open-warehouse scenario + e2e-dispatch smoke test. **Blocking**; runs on pushes to `main` | ubuntu-latest |
| `harness-nat` | Docker test harness — nat-office, enterprise-dmz, mobile-cgnat. **Non-blocking** (`continue-on-error`); runs on pushes to `main` | ubuntu-latest |

All blocking jobs must pass before merging.

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
| `gang` binary | Compiles | `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass (337 passed, 1 ignored) |
| `gang relay` command | Validated | Starts libp2p relay server, generates identity, listens on TCP+QUIC, prints multiaddrs |
| `gang agent` command | Validated | Creates data dir, generates identity, runs until Ctrl+C |
| `gang test-archetype` CLI | Fixed | Checks for docker + docker compose, cleans up previous runs, runs connectivity checks, provides teardown instructions |
| Compose: open-warehouse | Ready | 3 services (relay, robot, operator) on flat 172.20.0.0/24 |
| Compose: nat-office | Ready | 5 services (relay, robot, operator, 2 NAT gateways), 3 networks |
| Compose: enterprise-dmz | Ready | 4 services (relay, robot, operator, firewall), TCP 443 only + netem |
| Compose: mobile-cgnat | Ready | 5 services (relay, robot, operator, inner NAT, outer CGNAT), netem jitter/loss |
| `run-tests.sh` | Created | Builds base image, runs each scenario, checks containers + processes + connectivity + network rules |

All Dockerfile and compose file changes have been validated structurally. The
network archetype scenarios and the `e2e-dispatch` scenario are built and their
topologies verified; see the end-to-end status note below for what the
`e2e-dispatch` scenario asserts today.

**Changes made to fix the test harness:**

1. **Relay nodes** now run `gang relay --port <port>` instead of `gang agent` -- the relay command starts the libp2p circuit relay v2 server, which is what NAT'd clients need to connect through.
2. **Operator nodes** now run `gang agent --data-dir /data` instead of `gang demo` -- the demo command exits immediately after running (it creates a throwaway local agent), which causes the container to stop.
3. **Enterprise DMZ relay** listens on port 443 (`gang relay --port 443`) to match the firewall's TCP 443 outbound-only rule.
4. **Dockerfile.base** copies only `Cargo.toml`, `Cargo.lock`, and `crates/` (not the full repo), removes `rust-toolchain.toml` to avoid the wasm32-wasip2 target requirement, and pre-creates `/data`.
5. **`.dockerignore`** added to exclude `target/` (15 GB), `.git/`, and other non-essential directories from the Docker build context.
6. **`gang test-archetype` CLI** now validates both `docker` and `docker compose` availability, tears down leftover containers before starting, runs archetype-specific connectivity checks, and cleans up on failure.
7. **`run-tests.sh`** created as comprehensive test runner with 6 checks per scenario: container state, relay process, robot process, relay startup logs, archetype connectivity, and archetype network rules.

### Expected performance targets

| Metric | Open Warehouse | NAT'd Office | Enterprise DMZ | Mobile/CGNAT |
|--------|---------------|---------------|----------------|--------------|
| Connection establishment | <100ms | <2s (relay) + DCUtR upgrade | <3s (relay on 443) | <5s (double relay) |
| Steady-state RTT | <1ms (loopback) | <5ms (direct after DCUtR) | ~10ms (+5ms TLS inspect) | ~140ms (50±30 + 20±10 cumulative) |
| Control message roundtrip | <5ms | <15ms | <25ms | <300ms |
| Bulk transfer throughput | >100 MB/s (loopback) | >50 MB/s (direct) | >10 MB/s (relay) | >1 MB/s (relay + loss) |
| Reconnection time | <500ms | <3s | <5s | <10s |
| DCUtR upgrade | N/A | Expected success | Expected failure (UDP blocked) | Expected failure (symmetric NAT) |

> **Note:** These are expected targets based on Docker-compose netem parameters. Run `./test-harness/run-tests.sh` on a host with Docker to collect real numbers.

### End-to-end dispatch status

The `e2e-dispatch` scenario is a **connectivity smoke test**: it verifies that
the relay, robot agent, and operator containers start, that the operator can
reach the relay and the robot can register a circuit reservation, and that the
CLI resolves a registered peer. A **full remote deploy/invoke round-trip over
the relay is pending ADR-020 Phase 32** (operator remote dispatch and the agent
serve loop are still WIP). Until then, deploy/invoke are validated on the local
fallback path (`gang demo` and the unit/integration suites).

### Running the scenarios (requires Docker)

On a host with Docker, run:

```bash
cd /path/to/ganglion
./test-harness/run-tests.sh
```

Expected results per scenario:
- **open-warehouse:** operator can ping robot at 172.20.0.20 (flat L2)
- **nat-office:** robot can reach gateway at 192.168.1.1, NAT DROP rules present
- **enterprise-dmz:** robot can reach firewall at 172.16.10.1, TCP 443 rule present
- **mobile-cgnat:** robot can reach inner NAT at 10.64.0.1, netem qdisc active

## Known limitations

- WebTransport and WebRTC transports are not available on native targets — no libp2p release (including 0.56, which the project is on) ships native WebTransport or WebRTC. Config flags and capability reporting are in place for when a future libp2p release adds native support. The v0.2 design spec success criterion ("HTTPS/443-only egress operator can reach a robot via WebTransport") is blocked by this upstream dependency.

- Relay-mediated remote dispatch (`gang deploy`/`run`/`caps` to a remote robot) is WIP (ADR-020 Phase 32); the `e2e-dispatch` scenario is a connectivity smoke test, not a full round-trip.
- Docker test scenarios verify network topology and reachability; full end-to-end protocol flow testing requires live relay connectivity
- Regulated facility (air-gapped) archetype is not Docker-testable — requires physical sneakernet
- The `gang logs` and `gang list` commands require relay connectivity (not yet wired)
- Registry is local-only — distributed registry synchronization is planned for a future version
- The relay and agent nodes do not yet auto-discover each other within Docker scenarios; this requires passing the relay's multiaddr to the robot and operator containers (planned for the next harness iteration)
