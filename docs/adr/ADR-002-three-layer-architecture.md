# ADR-002: Three-layer architecture

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at design phase)

## Context

Remote robot tooling systems typically use one of two patterns:

1. **Flat agent:** A single privileged process on the robot handles connectivity, tool execution, and hardware access. Simple but high blast radius — a bug in a diagnostic tool can crash the agent and sever connectivity.
2. **Microservice mesh:** Each concern runs as an independent service. Flexible but operationally complex for edge devices with constrained resources.

Ganglion needs to run on robots in hostile networks (NAT, firewalls, CGNAT) while executing operator-supplied diagnostic tools that should never have direct hardware access.

## Decision

Organize the system into three layers with distinct trust boundaries:

1. **Layer 1 — Connectivity (native):** libp2p transport, identity, relay, NAT traversal. Runs as a long-lived native process. Stateless with respect to application logic. Must stay up when everything else is broken.
2. **Layer 2 — Tool Execution (WASM):** Wasmtime component runtime hosting signed, sandboxed tools. Tools declare capabilities; undeclared access traps. Fuel metering and epoch deadlines prevent runaway execution.
3. **Layer 3 — Native Brokers:** Small privileged processes mediating WASM access to system resources (ROS 2, filesystem, processes, network). Each broker implements `CapabilityBroker` trait. Default-deny policy engine.

## Consequences

- **Positive:** A misbehaving tool cannot crash connectivity (Layer 1 is independent of Layer 2).
- **Positive:** Tools cannot access resources they didn't declare (sandbox + policy engine).
- **Positive:** Each layer can be tested independently — `gang-libp2p` doesn't need ROS 2, `gang-wasm-host` doesn't need network connectivity.
- **Negative:** Cross-layer integration is more complex. The WASM host must correctly wire broker host functions, and the policy engine must be consulted on every broker call.
- **Negative:** Three layers means three failure modes. Debugging requires understanding which layer failed.
