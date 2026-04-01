# ADR-006: WASM component model for tool execution

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

Operator-supplied diagnostic tools run on robots inside customer networks. These tools must be:
- Sandboxed: no ambient authority over the host system
- Portable: run on any architecture the robot agent supports
- Auditable: signed, versioned, with declared capabilities
- Resource-bounded: cannot consume unbounded CPU, memory, or wall-clock time

Options considered:

1. **Native plugins (shared libraries):** Fast but no sandbox. A crash in a plugin crashes the agent.
2. **Container-based isolation (Docker/OCI):** Strong isolation but heavy for edge devices. Startup latency is significant.
3. **WASM (Wasmtime component model):** Near-native performance, strong sandboxing, portable bytecode, fuel metering for CPU bounds, epoch interrupts for wall-clock deadlines.
4. **Scripting (Lua/Python):** Easy to author but weaker sandboxing guarantees and no standardized capability model.

## Decision

Use the WebAssembly Component Model via Wasmtime. Components are compiled to `.wasm`, signed with Ed25519, and shipped with a manifest declaring required capabilities. The runtime enforces:

- **Fuel metering:** Each invocation gets a fuel budget. Exceeding it traps the component.
- **Epoch-based deadlines:** Wall-clock timeout via Wasmtime's epoch interruption mechanism.
- **No ambient authority:** Components cannot access the filesystem, network, or OS directly. All access goes through WIT-defined host functions backed by Layer 3 brokers.

## Consequences

- **Positive:** Components cannot escape the sandbox. A buggy tool cannot crash connectivity or access undeclared resources.
- **Positive:** The same `.wasm` binary runs on x86_64 and aarch64 robots without recompilation.
- **Positive:** Fuel + epoch provide hard bounds on resource consumption, critical for unattended robots.
- **Negative:** The WASM component model ecosystem is still maturing. Toolchain support for languages beyond Rust and C/C++ is limited.
- **Negative:** Serialization overhead at the WASM boundary (host function calls pass `list<u8>` for complex types).
- **Negative:** Debugging WASM components is harder than debugging native code — limited debugging tooling compared to native binaries.
