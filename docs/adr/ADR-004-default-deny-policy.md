# ADR-004: Default-deny policy engine

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

WASM components running on robots request access to system resources (ROS topics, files, processes, network). The system must decide which requests to allow. Two models:

1. **Default-allow with blocklist:** Everything is permitted unless explicitly blocked. Easier to get started but dangerous — a new capability group is open by default until someone remembers to restrict it.
2. **Default-deny with allowlist:** Nothing is permitted unless explicitly allowed. Safer but requires upfront policy configuration.

Ganglion operates in environments where robots are deployed inside customer networks. A policy failure means an operator-supplied tool could access resources on a customer's network that were never intended to be exposed.

## Decision

The policy engine is default-deny. Every broker operation is checked against the policy before execution. A missing policy rule means access is denied. Policies use glob patterns for flexible matching (e.g., `/cmd_vel*` allows `/cmd_vel` and `/cmd_vel_mux`).

Policy rules specify:
- Capability group (ros, fs, logs, diagnostics, artifacts, process, network, metrics)
- Resource pattern (glob)
- Access level (read-only or read-write)
- Optional peer authorization (which peer IDs may invoke)

Policies are stored as TOML and support roundtrip serialization.

## Consequences

- **Positive:** New capability groups are secure by default — no access until a policy rule is added.
- **Positive:** Audit trail can record both grants and denials, making policy debugging straightforward.
- **Positive:** Glob patterns are familiar to operators (shell-like) and expressive enough for topic/path hierarchies.
- **Negative:** First-run experience requires policy configuration before any tool can do useful work. Mitigated by `gang capability scaffold` generating a starter policy.
- **Negative:** Overly broad patterns (e.g., `/**`) can effectively disable the policy engine. Linting for overly permissive patterns is planned.
