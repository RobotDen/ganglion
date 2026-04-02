# Validation results

Test results and validation status for Ganglion v0.4.0.

## Test environment

- **Host:** macOS / Linux
- **Ganglion version:** v0.4.0
- **Rust:** 1.85+
- **CI:** GitHub Actions (ubuntu-latest + macos-latest matrix)

## Unit test summary

158 tests across 9 crates, all passing:

| Crate | Tests | Coverage areas |
|-------|-------|---------------|
| gang-core | 46 | Identity (keypair gen, persist, sign/verify, registry), messages (CBOR framing, varint), manifests (sign, verify, tamper detection, hash verification, v2 fields, schema version), policy (default-deny, patterns, peer auth, TOML roundtrip, process/network/metrics groups, read-write required for param-set), audit (write/read, rotation), artifacts (CID determinism, dedup, chunking, LRU eviction, persist/reload), registry (publish, search, tags, versions, persist/reload, remove) |
| gang-ros | 77 | Diagnostics broker (system info, network, processes), filesystem broker (read/write/list/stat, pattern gating, symlink jail, write-to-new-file traversal denial, symlink parent jail, nonexistent parent denial), log stream broker (source enumeration, pattern filtering), ROS broker (check_access exact/glob/wildcard/denied/read-only/read-write/empty patterns/first-match, topic subscribe with/without rosbridge, service call with/without rosbridge, param get/set with/without rosbridge, access denied propagation, unsupported ops, capability group), robot agent (deploy, invoke, trust verification), archetype detection (classify all 5 archetypes, display, recommendations), process broker (allowlist exact/glob/wildcard, spawn echo, denied command, handle request, unsupported op), network probe broker (DNS lookup, port check, traceroute, handle request, unsupported op), metrics broker (emit, batch, ring buffer eviction, drain, unsupported op) |
| gang-wasm-host | 10 | WIT interface parsing, component runtime setup, fuel metering |
| gang-libp2p | 0 | Integration-tested via test-harness scenarios |
| gang-cli | 0 | Integration-tested via `gang demo`, `gang status`, and manual CLI exercises |
| gang-capability-diagnostics | 0 | WIP — reference capability |
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
| v0.5.0 | 32 | 158 |

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

*To be populated after running scenarios on target hardware. Run:*

```bash
gang test-archetype open-warehouse
gang test-archetype nat-office
gang test-archetype enterprise-dmz
gang test-archetype mobile-cgnat
```

## Known limitations (v0.4)

- WASM component runtime host-function wiring is partially complete — capabilities can be loaded and fuel-metered, but full broker integration requires additional glue code
- Docker test scenarios verify network topology and reachability; full end-to-end protocol flow testing requires the WASM host integration
- Regulated facility (air-gapped) archetype is not Docker-testable — requires physical sneakernet
- The `gang logs` and `gang list` commands require relay connectivity (not yet wired)
- Registry is local-only — distributed registry synchronization is planned for a future version
