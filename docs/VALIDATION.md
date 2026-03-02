# Validation results

Test harness results from running Ganglion across four simulated network archetypes.

## Test environment

- **Host:** macOS / Linux with Docker
- **Ganglion version:** v0.1.0-dev
- **Rust:** 1.85+
- **Base image:** debian:bookworm-slim with iproute2, iptables, networking tools

## Unit test coverage

42 tests across all crates, all passing:

| Crate | Tests | Coverage areas |
|-------|-------|---------------|
| gang-core | 23 | Identity (keypair gen, persist, sign/verify, registry), messages (CBOR framing, varint), manifests (sign, verify, tamper detection, hash verification), policy (default-deny, patterns, peer auth), audit (write/read, rotation) |
| gang-ros | 19 | Diagnostics broker (system info, network, processes), filesystem broker (read/write/list/stat, pattern gating, symlink jail), log stream broker (source enumeration, pattern filtering), robot agent (deploy, invoke, trust verification) |

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

**Topology:** Two separate LANs (192.168.1.0/24, 192.168.2.0/24) behind independent NAT gateways, relay on shared internet (10.0.0.0/24). Internal networks marked `internal: true` — no direct external access.

| Metric | Expected | Notes |
|--------|----------|-------|
| Connection method | Relay, then DCUtR upgrade | Both sides behind NAT |
| NAT type | Endpoint-independent (MASQUERADE) | DCUtR should succeed |
| Inbound connections | Blocked | iptables DROP on NEW from external |

### Scenario 3: Enterprise DMZ

**Topology:** Robot on isolated VLAN (172.16.10.0/24) behind restrictive firewall. Only TCP 443 outbound permitted. Operator has direct internet (10.1.0.0/24). 5ms netem delay simulating TLS inspection overhead.

| Metric | Expected | Notes |
|--------|----------|-------|
| Connection method | Relay only | QUIC (UDP) blocked |
| Allowed ports | TCP 443 outbound only | All others dropped |
| Additional latency | +5ms | TLS inspection simulation |
| DCUtR | Will fail | Firewall blocks UDP |

### Scenario 4: Mobile/CGNAT

**Topology:** Robot behind double NAT — inner (10.64.0.0/24 → 100.64.0.0/24) and outer CGNAT (100.64.0.0/24 → 10.2.0.0/24). Symmetric NAT on outer layer (`--random`). Mobile network conditions simulated: 50ms±30ms latency, 2% packet loss on inner; 20ms±10ms, 0.5% loss on outer.

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

*Or use the runner script directly:*

```bash
./test-harness/run-scenario.sh open-warehouse
```

## Known limitations (v0.1)

- WASM component runtime not yet integrated — capabilities run as native broker calls
- Scenarios verify network topology and reachability, not full Ganglion protocol flow
- Measured numbers will be filled in once libp2p transport is wired into the agent process
- Regulated facility (air-gapped) archetype is not Docker-testable — requires physical sneakernet
