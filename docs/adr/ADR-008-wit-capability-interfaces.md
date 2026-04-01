# ADR-008: WIT-defined capability interfaces

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

WASM components need a typed interface to request broker operations. Options:

1. **Custom binary protocol:** Flexible but requires custom serialization and is opaque to tooling.
2. **gRPC/Protobuf:** Well-tooled but heavyweight for in-process host function calls.
3. **WIT (WebAssembly Interface Types):** The component model's native IDL. Strongly typed, composable, generates bindings for multiple languages.

## Decision

Define all capability interfaces in WIT (`ganglion.wit`). Each capability group is a separate WIT interface under the `ganglion:capability@0.4.0` package:

- `ros-interface` — topic subscribe, service call, param get
- `logs-stream` — source enumeration, log streaming
- `fs-bounded` — path-gated file operations
- `diagnostics-collect` — system information collection
- `artifacts-publish` — content-addressed artifact storage
- `process-spawn` — allowlisted subprocess execution
- `network-probe` — structured network probing
- `metrics-emit` — ring-buffered metrics emission

Each interface uses `result<T, string>` return types for error propagation. Complex data crosses the boundary as `list<u8>` (serialized).

## Consequences

- **Positive:** WIT is the standard IDL for the component model. Tools like `wit-bindgen` can generate host and guest bindings.
- **Positive:** Each interface is independently versionable. A component targeting `ros-interface@1.0` doesn't break when `metrics-emit` advances to `@2.0`.
- **Positive:** The manifest's capability declarations map directly to WIT interface names, making policy enforcement straightforward.
- **Negative:** WIT tooling is still evolving. Some edge cases (nested records, resource types) require workarounds.
- **Negative:** `list<u8>` as a serialization envelope loses the type safety that WIT otherwise provides. Future versions should use WIT records for structured data.
