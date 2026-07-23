# Ganglion Documentation

Index of the Ganglion documentation set. Start with the
[project README](../README.md) for an overview, then dive in here.

## Getting started

- [QUICKSTART.md](QUICKSTART.md) — zero to running `gang` in a few minutes.
- [CLI_REFERENCE.md](CLI_REFERENCE.md) — every command, flag, and example.

## Understanding the system

- [ARCHITECTURE.md](ARCHITECTURE.md) — the three-layer architecture, crate map,
  data flows, and current crate dependency graph.
- [SECURITY.md](SECURITY.md) — threat model, trust boundaries, and the
  implemented security mechanisms (fail-closed policy/trust, replay protection,
  hash-chained audit log, broker hardening, unified peer-id derivation).
- [NETWORK_ARCHETYPES.md](NETWORK_ARCHETYPES.md) — the five network archetypes,
  detection probes, and transport recommendations. See also the one-page
  [decision flowchart](decision-flowchart.svg).

## Building capabilities

- [CAPABILITY_AUTHOR_GUIDE.md](CAPABILITY_AUTHOR_GUIDE.md) — authoring
  capabilities in Rust, C++, Python, and Go. See also the language
  [examples](../examples/README.md).

## Releases and upgrading

- [CHANGELOG.md](../CHANGELOG.md) — release history.
- [MIGRATION-v2.md](MIGRATION-v2.md) — upgrade guide for the v2.0.0 breaking
  changes.
- [VALIDATION.md](VALIDATION.md) — test results, CI pipeline, and network
  scenario status.

## Design history and decisions

- [adr/](adr/README.md) — Architecture Decision Records (ADR-001 … ADR-020).
- [IMPLEMENTATION.md](IMPLEMENTATION.md) — implementation plan and progress
  (includes historical sections superseded by ADR-001).
- [DesignSpec.md](DesignSpec.md) — the original (historical) design
  specification.
