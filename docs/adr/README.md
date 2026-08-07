# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the Ganglion project.

## Format

Each ADR follows a consistent format:

- **Title** — short imperative-mood description
- **Status** — Accepted, Proposed, Deprecated, or Superseded
- **Context** — the forces at play
- **Decision** — what we decided
- **Consequences** — what follows from the decision

## Index

### Foundational decisions (v0.1)

| # | Title | Status |
|---|-------|--------|
| 001 | [Monorepo workspace](ADR-001-monorepo-workspace.md) | Accepted |
| 002 | [Three-layer architecture](ADR-002-three-layer-architecture.md) | Accepted |
| 003 | [Ed25519 identity model](ADR-003-ed25519-identity.md) | Accepted |
| 004 | [Default-deny policy engine](ADR-004-default-deny-policy.md) | Accepted |
| 005 | [Protocol-agnostic transport adapter](ADR-005-transport-adapter-trait.md) | Accepted |
| 006 | [WASM component model for tool execution](ADR-006-wasm-component-model.md) | Accepted |
| 007 | [Simplified WASM capability linking](ADR-007-simplified-wasm-linking.md) | Accepted |
| 008 | [WIT-defined capability interfaces](ADR-008-wit-capability-interfaces.md) | Accepted |
| 009 | [Outbound-initiated connectivity](ADR-009-outbound-initiated.md) | Accepted |

### Evolution decisions (v0.2–v0.4)

| # | Title | Status |
|---|-------|--------|
| 010 | [Content-addressed artifact store](ADR-010-content-addressed-artifacts.md) | Accepted |
| 011 | [Manifest schema v2 with backward compatibility](ADR-011-manifest-schema-v2.md) | Accepted |
| 012 | [Network archetype detection](ADR-012-network-archetype-detection.md) | Accepted |
| 013 | [Rosbridge over rclrs for ROS 2 integration](ADR-013-rosbridge-over-rclrs.md) | Accepted |
| 014 | [Rust edition 2024](ADR-014-rust-edition-2024.md) | Accepted |

### Review findings (v0.5 — implemented)

| # | Title | Status |
|---|-------|--------|
| 015 | [Fix filesystem broker symlink jail for writes](ADR-015-fix-symlink-jail-writes.md) | Accepted |
| 016 | [Implement ROS broker stubbed operations](ADR-016-implement-ros-stubs.md) | Accepted |
| 017 | [Add param-set to WIT ros-interface](ADR-017-wit-param-set.md) | Accepted |
| 018 | [Document CLI stubbed commands](ADR-018-document-cli-stubs.md) | Accepted |
| 019 | [ROS broker test coverage](ADR-019-ros-broker-test-coverage.md) | Accepted |

### Remote dispatch (v0.6)

| # | Title | Status |
|---|-------|--------|
| 020 | [Remote dispatch via control protocol and e2e test](ADR-020-remote-dispatch-and-e2e-test.md) | Accepted; partially implemented |

### Onboarding (v0.7)

| # | Title | Status |
|---|-------|--------|
| 021 | [Pairing-token enrollment (`gang pair` / `gang join`)](ADR-021-pairing-token-enrollment.md) | Accepted; implemented |

### Presence & streaming (v0.8)

| # | Title | Status |
|---|-------|--------|
| 022 | [Robot→operator event subscription layer](ADR-022-event-subscription-layer.md) | Accepted; implemented (default transport superseded by ADR-024; poll retained as fallback) |

### Live dashboard (v0.9)

| # | Title | Status |
|---|-------|--------|
| 023 | [`gang tui` live fleet dashboard](ADR-023-tui-dashboard.md) | Accepted; implemented |
| 024 | [True server-push event stream (with poll fallback) over libp2p-stream](ADR-024-event-push-stream.md) | Accepted; implemented |
