# Network Archetypes

Ganglion is designed around five real-world network environments that robots are deployed into. Each archetype has distinct constraints that determine which transport strategy will work. This document describes each archetype, how Ganglion detects it, and what transport configuration it recommends.

## Why archetypes matter

Robots are deployed into networks you don't own and can't configure. A robot on a warehouse floor has different connectivity than one behind an enterprise firewall or on a cellular modem. Trying a single connectivity strategy for all environments will fail — direct QUIC won't work through CGNAT, and relay-only adds unnecessary latency on an open LAN.

Ganglion's archetype detection automates this: probe the network, classify it, apply the right transport configuration.

```bash
gang diagnose          # Probe local network
gang diagnose robot-42 # Probe a remote robot's network
```

For a one-page visual of the archetype → transport strategy → relay-requirement
mapping, see the [decision flowchart](decision-flowchart.svg).

## The five archetypes

### 1. Open warehouse

**Characteristics**: Flat L2 network, permissive DHCP, no NAT, no egress controls. Common in small warehouses, labs, and development environments.

**Detection signals**:
- Internet connectivity: yes
- NAT: no (public or flat private gateway)
- Multicast: yes
- Outbound ports: unrestricted
- CGNAT: no

**Transport strategy**:
- Direct QUIC connection (lowest latency)
- No relay needed
- Multicast discovery available for peer finding
- TCP fallback if QUIC is blocked by local equipment

**Typical latency**: <1ms (LAN)

---

### 2. NAT'd office

**Characteristics**: Single consumer NAT (home router, small office router), no inbound port forwarding, DHCP with private addressing. The most common deployment environment.

**Detection signals**:
- Internet connectivity: yes
- NAT: yes (private gateway, 192.168.x.x or 10.x.x.x)
- Multicast: usually yes (on LAN segment)
- Outbound ports: unrestricted
- CGNAT: no

**Transport strategy**:
1. Robot dials out to a relay server
2. Operator connects to the robot via the relay
3. DCUtR hole-punch upgrades the connection to direct QUIC
4. After hole-punch, relay is no longer in the data path

**Typical latency**: 20-50ms (through relay), <5ms (after DCUtR upgrade)

**Key insight**: Endpoint-independent NAT (the common case with consumer routers) allows DCUtR to succeed. The relay bootstraps the connection; the hole-punch eliminates relay overhead.

---

### 3. Enterprise DMZ

**Characteristics**: VLAN isolation, restricted outbound ports (only 80/443 allowed), TLS inspection proxy, centrally managed DNS. Common in hospitals, factories with IT oversight, and corporate offices.

**Detection signals**:
- Internet connectivity: yes (through proxy)
- NAT: yes
- Multicast: no (inter-VLAN blocked)
- Outbound ports: restricted (non-443 ports blocked)
- DNS: may be intercepted or filtered
- CGNAT: no

**Transport strategy**:
- Relay on TCP 443 is required (QUIC/UDP likely blocked)
- Configure relay to listen on port 443 to pass through firewall rules
- DCUtR will likely fail (firewall blocks unsolicited inbound)
- Plan for relay-only operation
- Expect +5-20ms additional latency from TLS inspection

**Typical latency**: 30-80ms (relay + TLS inspection overhead)

**Operational note**: The relay must be reachable on TCP 443. If the enterprise proxies all 443 traffic through a TLS inspection appliance, the relay must present a valid TLS certificate. WebSocket transport over 443 can also work in these environments.

---

### 4. Regulated facility

**Characteristics**: Air-gapped or physically isolated network, no outbound internet, sneakernet transfers only. Common in defense, nuclear, and high-security manufacturing environments.

**Detection signals**:
- Internet connectivity: no
- All other probes: typically fail or return inconclusive

**Transport strategy**:
- No network connectivity — use offline signed bundles
- Pre-sign capabilities on an external machine
- Transfer via USB or approved sneakernet process
- The robot agent validates signatures locally without network access

**Workflow**:
```bash
# On the external machine (with internet)
gang sign diagnostics.wasm --name diagnostics --version 0.1.0

# Transfer diagnostics.wasm + diagnostics.manifest.cbor via USB

# On the robot (air-gapped)
gang deploy local diagnostics.wasm --manifest diagnostics.manifest.cbor
gang run local diagnostics
```

**Key insight**: Ganglion's manifest signing and policy engine work entirely offline. The robot agent verifies signatures against its local trust store and evaluates policy without any network calls.

---

### 5. Mobile/CGNAT

**Characteristics**: Symmetric NAT, carrier-grade NAT (CGNAT), IP rotation, intermittent connectivity, variable latency. Common with cellular modems (4G/5G), satellite links, and mobile robots.

**Detection signals**:
- Internet connectivity: yes (intermittent)
- NAT: yes
- CGNAT: yes (address in 100.64.0.0/10 range)
- Outbound ports: variable
- Multicast: no

**Transport strategy**:
- Relay-only connectivity (symmetric NAT defeats hole-punching)
- Aggressive reconnection logic (connections will drop)
- Chunked transfers for large payloads (connection may interrupt)
- Expect variable latency (50ms+ base) and intermittent drops

**Typical latency**: 50-200ms (relay + cellular), with spikes during handoffs

**Key insight**: DCUtR will not work with symmetric NAT. The relay must remain in the data path for the entire session. Design payloads to be resumable and idempotent.

## Archetype detection probes

Ganglion runs six probes to classify the network:

| Probe | What it checks | Method |
|-------|---------------|--------|
| `internet_connectivity` | Outbound internet access | DNS resolution of `dns.google`, fallback to ICMP ping to `8.8.8.8` |
| `nat_status` | Whether behind NAT | Checks default gateway for private address ranges (192.168.x, 10.x, 172.16-31.x) |
| `multicast` | Multicast-capable interfaces | Checks interface flags for `MULTICAST` capability |
| `outbound_ports` | Non-443 port accessibility | TCP connect to `8.8.8.8:53` (DNS over TCP) |
| `dns_behavior` | DNS filtering or interception | TXT record query for `dns.google` |
| `symmetric_nat` | CGNAT address ranges | Checks interface addresses for 100.64.0.0/10 range |

### Classification logic

```
No internet?              → Regulated facility (0.8 confidence)
CGNAT detected?           → Mobile/CGNAT (0.85 confidence)
NAT + ports blocked?      → Enterprise DMZ (0.8 confidence)
NAT + multicast?          → NAT'd office (0.75 confidence)
NAT only?                 → NAT'd office (0.7 confidence)
Multicast + ports open?   → Open warehouse (0.85 confidence)
Default fallback          → NAT'd office (0.5 confidence)
```

## Transport recommendation matrix

| Archetype | Primary transport | Relay needed | DCUtR | Multicast discovery |
|-----------|------------------|-------------|-------|-------------------|
| Open warehouse | Direct QUIC | No | N/A | Yes |
| NAT'd office | QUIC (after DCUtR) | Yes (bootstrap) | Yes | Yes (LAN) |
| Enterprise DMZ | TCP 443 (relay) | Yes (permanent) | No | No |
| Regulated facility | Offline (USB) | No | No | No |
| Mobile/CGNAT | Relay (any) | Yes (permanent) | No | No |

## Capability: network-archetype

The `gang-capability-network-archetype` crate provides a WASM-deployable version of archetype detection that can run as a capability on a remote robot:

- Runs the network probe broker's structured probing primitives
- Computes a weighted connectivity score (0-100)
- Classifies into one of the five archetypes
- Generates transport recommendations
- Returns a structured `ArchetypeReport` with all probe results

```bash
gang deploy robot-42 gang-capability-network-archetype.wasm
gang run robot-42 network-archetype
```

## Testing with Docker

The `test-harness/` directory contains Docker Compose configurations that simulate each archetype using `tc`/`netem` for traffic shaping and `iptables` for firewall rules:

```bash
gang test-archetype open-warehouse    # Flat network, no restrictions
gang test-archetype nat-office        # NAT with hole-punch opportunity
gang test-archetype enterprise-dmz    # VLAN isolation, port 443 only
gang test-archetype mobile-cgnat      # Symmetric NAT, latency, jitter, loss
```

Each scenario starts containers for the relay, robot agent, and simulated network environment. The test validates that Ganglion successfully establishes connectivity and can deploy and invoke capabilities under the archetype's constraints.
