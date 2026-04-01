# ADR-005: Protocol-agnostic transport adapter

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

Design principle #1: "Protocol-agnostic core, opinionated defaults." The core specification should define adapter interfaces; libp2p is the recommended default transport but not the only valid one.

Some deployment environments may require alternatives:
- Enterprise networks that mandate MQTT or AMQP brokers
- Air-gapped facilities where USB sneakernet is the only transport
- Environments where libp2p's DHT traffic is blocked by network policy

## Decision

Define a `TransportAdapter` trait in `gang-libp2p` that abstracts peer-to-peer communication. The libp2p implementation is the default and only current adapter. The trait surface covers: connect, send, receive, peer discovery, and connection status.

The core types in `gang-core` (messages, manifests, policy, audit) have zero dependency on `gang-libp2p` or any transport implementation. The CLI crate wires a concrete adapter at startup.

## Consequences

- **Positive:** `gang-core` can be tested without any network stack.
- **Positive:** Alternative transports (MQTT bridge, HTTP polling, sneakernet) can be added without modifying core types.
- **Positive:** The dependency rule (`gang-core` depends on nothing) is enforced by Cargo's workspace dependency graph.
- **Negative:** The trait surface was designed around libp2p's connection model. A radically different transport (e.g., one-way broadcast) may require trait evolution.
- **Negative:** Only one adapter exists today, so the abstraction is tested by exactly one implementation. The trait may need refinement when a second adapter is built.
