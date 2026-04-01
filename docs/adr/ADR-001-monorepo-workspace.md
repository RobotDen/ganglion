# ADR-001: Monorepo Cargo workspace

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

The original design specification (IMPLEMENTATION.md §Repository Layout) planned four separate repositories:

1. `ganglion-spec` — specification documents
2. `ganglion-libp2p` — connectivity layer
3. `ganglion-ros` — ROS 2 integration
4. `ganglion` — meta-repo tying them together

This mirrors a pattern common in large open-source ecosystems where independent release cadences and team ownership boundaries justify separate repos.

However, Ganglion is authored by a single team, all crates share a common `gang-core` dependency, and the v0.1–v0.4 development cycle involves tight cross-crate iteration (e.g., adding a new broker touches `gang-core`, `gang-ros`, `gang-wasm-host`, and `gang-cli` simultaneously).

## Decision

Use a single Cargo workspace with all crates under `crates/`. Specifications and documentation live in `docs/` within the same repository.

## Consequences

- **Positive:** Atomic commits across crate boundaries. Single CI pipeline. No version coordination overhead. `cargo test` validates the entire workspace.
- **Positive:** Contributors can clone one repo and have the full context.
- **Negative:** Cannot release individual crates to crates.io with independent version numbers without workspace-level version management tooling (e.g., `cargo-workspaces`).
- **Negative:** The repository will grow larger as capability crates are added. Mitigated by keeping capability crates small and well-scoped.
- **Migration path:** If independent release cadences become necessary (e.g., `gang-core` stabilizes while `gang-ros` iterates rapidly), extract crates into separate repos at that point. The `CapabilityBroker` trait and WIT interfaces provide clean split points.
