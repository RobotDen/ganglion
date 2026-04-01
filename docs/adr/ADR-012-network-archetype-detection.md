# ADR-012: Network archetype detection

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.4)

## Context

Ganglion must work across radically different network environments. Rather than requiring operators to manually configure transport parameters, the system should detect the network environment and recommend an appropriate configuration.

The five archetypes (open warehouse, NAT'd office, enterprise DMZ, regulated facility, mobile/CGNAT) were identified from real-world robot deployment patterns in the FleetLink prospect research.

## Decision

Implement automated archetype detection via `gang diagnose`. The detection runs six network probes:

1. Direct connectivity test (can peers reach each other without relay?)
2. NAT type detection (endpoint-independent vs symmetric)
3. UDP availability (is QUIC possible?)
4. Outbound port restrictions (which ports are open?)
5. Latency and jitter measurement
6. Packet loss measurement

Each probe produces a structured result. A classifier maps the probe results to one of the five archetypes and generates transport recommendations (e.g., "use relay on TCP 443, disable DCUtR").

The `gang-capability-network-archetype` crate implements this as a WASM-compatible capability, demonstrating that diagnostic tools can themselves be sandboxed components.

## Consequences

- **Positive:** Operators get actionable transport configuration without understanding libp2p internals.
- **Positive:** The archetype model provides a shared vocabulary for discussing network constraints ("this is an enterprise DMZ deployment").
- **Positive:** Docker test scenarios (`gang test-archetype`) validate each archetype's behavior in CI.
- **Negative:** Real networks may not fit cleanly into five categories. The classifier must handle hybrid cases gracefully.
- **Negative:** Probe results are point-in-time. A network that changes characteristics (e.g., mobile robot moving between WiFi and cellular) may need re-detection.
