# ADR-013: Rosbridge over rclrs for ROS 2 integration

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

Ganglion's ROS 2 broker needs to interact with ROS 2 topics, services, and parameters on the robot. Two integration paths:

1. **rclrs (Rust ROS 2 client library):** Direct integration via the ROS 2 client library. Type-safe, no intermediate process, but requires ROS 2 to be installed and the correct message types compiled. Heavy build dependency. Ties the build to a specific ROS 2 distribution.
2. **rosbridge:** JSON-over-WebSocket protocol that any ROS 2 installation can serve via `rosbridge_server`. No build-time ROS 2 dependency. Works with any ROS 2 distribution. Looser coupling.

## Decision

Use rosbridge as the ROS 2 integration mechanism. The `RosBroker` checks for rosbridge availability at startup (`rosbridge_available` flag). When unavailable, ROS operations return `BrokerError::Unavailable` rather than crashing.

This keeps `gang-ros` buildable on any platform without ROS 2 installed, which is critical for:
- CI on standard GitHub Actions runners (no ROS 2)
- Developer machines that may not have ROS 2
- Cross-compilation for different robot architectures

## Consequences

- **Positive:** `cargo build` works on any machine. No ROS 2 build dependency.
- **Positive:** Works with any ROS 2 distribution (Humble, Iron, Jazzy) without recompilation.
- **Positive:** The rosbridge protocol is stable and well-documented.
- **Negative:** rosbridge adds a network hop and JSON serialization overhead compared to direct rclrs.
- **Negative:** rosbridge must be running on the robot — an additional process to manage.
- **Negative:** Some ROS 2 features (lifecycle nodes, QoS policies) are not exposed through rosbridge.
- **Future work:** Optional rclrs integration behind a feature flag for deployments where build-time ROS 2 availability is guaranteed and performance matters.
