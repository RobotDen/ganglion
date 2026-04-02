# ADR-016: Implement ROS broker stubbed operations

**Status:** Accepted (implemented)
**Date:** 2026-04-23

## Context

The `RosBroker` in `gang-ros/src/ros.rs` has two operations that are stubbed with "not yet implemented" errors:

- `BrokerOperation::ServiceCall` — checks access and rosbridge availability, then returns `BrokerError::Unavailable` with reason "service calls not yet implemented"
- `BrokerOperation::ParamGet` — checks access, then returns `BrokerError::Unavailable` with reason "parameter operations not yet implemented"

These stubs were acceptable for v0.1–v0.4 because:
1. Topic subscription (`TopicSubscribe`) is the most common diagnostic operation
2. The rosbridge integration strategy (ADR-013) was being validated
3. The WIT interface (`service-call`, `param-get`) and policy engine already support these operations

However, the `gang-capability-param-inspect` crate exists specifically for parameter operations, and it cannot function through the broker path until `ParamGet` is implemented.

## Decision

Implement `ServiceCall` and `ParamGet` via rosbridge WebSocket protocol in v0.5:

1. **`ParamGet`:** Send a `ros2param:get` request to rosbridge. Deserialize the response and return as `list<u8>`.
2. **`ServiceCall`:** Send a `call_service` request with the serialized payload. Handle timeouts and error responses from rosbridge.

Both operations already pass through `check_access()` and the policy engine. The implementation is limited to the rosbridge communication layer.

## Consequences

- **Positive:** Enables the `param-inspect` capability to work end-to-end through the broker.
- **Positive:** Completes the ROS interface contract defined in the WIT file.
- **Negative:** Requires a rosbridge WebSocket client in `gang-ros`. Consider `tungstenite` or `tokio-tungstenite` for async WebSocket.
- **Negative:** Service calls may have long timeouts. Need to integrate with the epoch-based deadline system.
- **Testing:** Integration tests require a running rosbridge instance. Use mock WebSocket server for unit tests.
