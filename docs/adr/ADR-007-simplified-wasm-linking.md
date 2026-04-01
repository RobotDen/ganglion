# ADR-007: Simplified WASM capability linking (v0.1–v0.4)

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

The full WASM component model linking flow requires:
1. Parse the component's WIT world definition
2. Generate host-function bindings for each declared interface
3. Wire each binding to the corresponding broker
4. Instantiate the component with only the declared imports

This is the correct long-term approach but requires significant glue code between the WIT interface definitions and the `CapabilityBroker` trait implementations. During early development (v0.1–v0.4), the focus was on proving the architecture — identity, policy, manifests, brokers — rather than production WASM integration.

## Decision

Use a simplified invocation path: instantiate the component with whatever imports the linker provides. Unlinked imports cause instantiation to fail, which maps to "undeclared capability" semantics. This is documented in `gang-wasm-host/src/runtime.rs` as an intentional simplification.

The brokers (`gang-ros`) are fully implemented and tested independently. The WASM host can load, fuel-meter, and epoch-limit components. The gap is the glue layer that routes WIT host function calls to the correct broker.

## Consequences

- **Positive:** Allowed v0.1–v0.4 to focus on proving the broker architecture, policy engine, and transport layer without blocking on WASM toolchain maturity.
- **Positive:** The "instantiation failure = missing capability" behavior is a reasonable approximation of the correct semantics.
- **Negative:** WASM components cannot currently call broker host functions at runtime. End-to-end tool execution requires the glue layer.
- **Negative:** The Docker test scenarios validate network topology but not full protocol flow through WASM.
- **Resolution:** v0.5 should implement the full WIT-to-broker binding generation. The binding surface is well-defined: 8 WIT interfaces × corresponding `CapabilityBroker` implementations.
