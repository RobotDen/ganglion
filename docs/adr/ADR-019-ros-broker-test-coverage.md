# ADR-019: ROS broker test coverage

**Status:** Accepted (implemented)
**Date:** 2026-04-23

## Context

The `RosBroker` in `gang-ros/src/ros.rs` has 46 tests across the `gang-ros` crate, but the `RosBroker` itself has zero unit tests. All 46 tests cover the other brokers: `DiagnosticsBroker`, `FsBroker`, `LogStreamBroker`, `ProcessBroker`, `NetworkProbeBroker`, and `MetricsBroker`.

Specifically untested in `RosBroker`:
- `check_access()` — pattern matching against allowed ROS patterns
- `handle_request()` — routing broker operations to the correct handler
- Topic subscription path (returns mock data when rosbridge unavailable)
- Service call and param-get stub behavior
- The `rosbridge_available` flag's effect on behavior

This gap exists because `RosBroker` was the first broker implemented (v0.1) before the testing patterns were established with later brokers, and its operations depend on rosbridge availability.

## Decision

Add unit tests for `RosBroker` in v0.5, covering:

1. **`check_access()` tests:**
   - Allowed pattern matches (exact, glob, wildcard)
   - Denied patterns (no matching rule)
   - Read-only vs read-write access levels
   - Empty pattern list (deny all)

2. **`handle_request()` tests:**
   - `TopicSubscribe` with rosbridge available/unavailable
   - `ServiceCall` stub behavior
   - `ParamGet` stub behavior
   - Unsupported operation rejection
   - Access denied propagation

3. **Constructor tests:**
   - Default `rosbridge_available = false`
   - Pattern configuration

Target: 12-15 tests, matching the pattern density of `ProcessBroker` (6 tests) and `NetworkProbeBroker` (5 tests) scaled for the larger API surface.

## Consequences

- **Positive:** Catches regressions when implementing ADR-016 (service call and param-get).
- **Positive:** Validates the access control path that protects ROS 2 resources.
- **Positive:** Establishes test patterns that future ROS-specific brokers can follow.
- **Negative:** Tests without a real rosbridge instance can only verify the broker's local logic, not the WebSocket protocol handling. Integration tests with a mock WebSocket server are needed separately.
