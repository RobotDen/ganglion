# ADR-020: Remote dispatch via control protocol and end-to-end validation

**Status:** Proposed
**Date:** 2026-04-24

## Context

Ganglion's architecture defines three roles (relay, robot agent, operator) communicating over libp2p via `/ganglion/control/1.0`. The infrastructure for this exists:

- **Message types:** `ControlMessage` enum with `DeployCapability`, `InvokeCapability`, `InvokeResult`, `ListCapabilities`, `Error`, `Presence` variants, CBOR-encoded with varint-prefixed framing (`gang-core::message`).
- **Transport:** `Libp2pTransportAdapter` with `request_response::Behaviour<GanglionCodec>`, 16 MiB length-prefixed codec, `dial()`, `listen()`, `handle_rpc_message()` dispatch.
- **Protocol constants:** `PROTOCOL_CONTROL`, `PROTOCOL_TOOL`, `PROTOCOL_BULK` defined in `gang-core::protocol`.
- **Relay server:** `gang relay` starts a circuit relay v2 server and prints its multiaddr.

However, two critical pieces are missing:

1. **Robot side:** `gang agent` creates a `RobotAgent` and waits for Ctrl+C. It never starts a libp2p transport, never dials a relay, and never calls `transport.listen()` to register a handler for incoming `ControlMessage` requests. There is no `serve()` method.

2. **Operator side:** `gang deploy` and `gang run` construct a *local* `RobotAgent` at `/tmp/gang-agent-{robot}` and call `deploy_capability()` / `invoke_capability()` directly in-process. They never dial a remote peer over libp2p.

The design spec (§4.2, §4.5) states that v0.1 should demonstrate deploy and invoke "across a simulated hostile network." The local-only path was acceptable for proving the broker architecture, but the project's thesis — hostile-network reachability — requires the relay-mediated remote path to be real.

There is also no end-to-end validation that stands up all three roles and exercises the full deploy→invoke→result flow. The Docker test harness validates network topology (ping, iptables rules, netem) but not application-level protocol exchange.

## Decision

Implement remote dispatch in v0.6 across four work areas:

### 1. Robot agent serve loop (`gang-ros`)

Add `RobotAgent::serve()` that:

- Takes a `&dyn TransportAdapter` and registers a handler on `PROTOCOL_CONTROL`.
- The handler deserializes incoming bytes as `ControlMessage` (using `gang_core::message::decode_message`).
- Routes by variant:
  - `DeployCapability { name, version, manifest_cbor, component_bytes }` → calls `self.deploy_capability()`, responds with `InvokeResult { status: Success, output: name.as_bytes() }` or `Error`.
  - `InvokeCapability { name, args, request_id }` → calls `self.invoke_capability()`, responds with `InvokeResult { request_id, status, output }`.
  - `ListCapabilities` → calls `self.list_capabilities()`, responds with `CapabilityList`.
  - `Presence` → logged, no response needed (one-way announcement).
- All responses serialized via `encode_message` and written to the stream.
- Errors (policy denied, unknown capability, signature failure) produce `ControlMessage::Error` with structured codes matching `InvokeStatus`.

### 2. Agent CLI startup with transport (`gang-cli`)

Modify `gang agent` to:

- Accept `--relay <multiaddr>` flag (required for remote mode, optional for local-only dev mode).
- Create `Libp2pTransportAdapter` with `relay_server: false` and the agent's identity.
- Dial the relay multiaddr (establishing the circuit reservation).
- Call `agent.serve(&transport)` to begin listening on the control protocol.
- Print the robot's peer ID and relay circuit multiaddr so the operator knows how to reach it.
- Continue running the transport event loop until Ctrl+C.

Without `--relay`, the agent behaves as today (local mode, no transport) for backward compatibility with `gang demo` and local testing.

### 3. Operator remote dispatch (`gang-cli`)

Modify `gang deploy` and `gang run` to:

- Accept `--peer <peer-id>` flag. When present, use network dispatch instead of local agent.
- Accept `--relay <multiaddr>` flag for relay-mediated connections (can default to the bootstrap relay constant in `gang-libp2p::config`).
- Create `Libp2pTransportAdapter` with the operator's identity.
- Dial the relay, then request a circuit to the robot's peer ID.
- Serialize the appropriate `ControlMessage` via `encode_message`.
- Send via `transport.send_request()` and await the response.
- Deserialize the response as `ControlMessage` (`InvokeResult`, `CapabilityList`, or `Error`).
- Print the result in the requested format (text or JSON).
- Shut down the transport after the operation completes.

When `--peer` is absent, fall back to the existing local-agent path (`/tmp/gang-agent-{robot}`), preserving the `gang demo` workflow.

### 4. End-to-end test scenario

A new Docker Compose scenario (`test-harness/e2e-dispatch/`) that validates the full flow:

**Topology:** Three containers on a flat bridge network (simplest case — proving protocol correctness, not network hostility, which the existing 4 scenarios cover).

- `relay` — runs `gang relay --port 4001`
- `robot` — runs `gang agent --relay /ip4/<relay-ip>/tcp/4001/p2p/<relay-peer-id> --data-dir /data`
- `operator` — runs a test script that:
  1. Waits for the relay and robot to be ready (poll relay's TCP port, then check robot's presence).
  2. Runs `gang deploy --peer <robot-peer-id> --relay <relay-multiaddr> /test-data/diagnostics.wasm`.
  3. Runs `gang run --peer <robot-peer-id> --relay <relay-multiaddr> diagnostics`.
  4. Asserts: exit code 0, output contains expected diagnostic fields.
  5. Runs `gang caps --peer <robot-peer-id> --relay <relay-multiaddr>`.
  6. Asserts: output lists `diagnostics`.

**Test payload:** `gang-capability-diagnostics` compiled to a WASM component (`.wasm`) and signed with a test keypair. The build step compiles the capability crate to `wasm32-wasip2`, runs `wasm-tools component new` to produce a component, and signs it with `gang sign`.

**Test runner:** `test-harness/e2e-dispatch/run-test.sh` builds the base image, compiles the test WASM component, starts the scenario, runs the operator script, checks results, tears down.

### 5. Reference WASM component build

Add a build target for `gang-capability-diagnostics` that produces a signed `.wasm` component:

- `Makefile` or `build.rs` in `crates/gang-capability-diagnostics/` that:
  1. `cargo build --target wasm32-wasip2 -p gang-capability-diagnostics`
  2. `wasm-tools component new target/wasm32-wasip2/release/gang_capability_diagnostics.wasm -o diagnostics.wasm`
  3. `gang sign diagnostics.wasm --name diagnostics --key <test-key>`
- This is the test payload for the e2e scenario and the reference artifact for capability authors.

## Consequences

### Positive

- **Proves the thesis.** The project's central claim — hostile-network reachability — is validated with an actual relay-mediated deploy and invoke, not just a local in-process simulation.
- **Uses existing infrastructure.** `ControlMessage`, `GanglionCodec`, `handle_rpc_message()`, `encode_message`/`decode_message`, `TransportAdapter` are all implemented and tested. The new code connects them; it doesn't replace them.
- **Backward compatible.** `gang deploy robot1 tool.wasm` (no `--peer` flag) continues to work as it does today. The local path remains the fast-iteration development workflow.
- **Validates the WASM component pipeline.** Building a real `.wasm` component from `gang-capability-diagnostics` proves the authoring pipeline works end-to-end, not just in unit tests.
- **CI-testable.** The flat-network e2e scenario doesn't require Docker networking features beyond a bridge network, so it runs on GitHub Actions ubuntu-latest without privilege escalation.

### Negative

- **WASM component compilation requires `wasm32-wasip2` target.** This target must be installed (`rustup target add wasm32-wasip2`) and `wasm-tools` must be available. CI must install these. The `rust-toolchain.toml` already declares the target but the Dockerfile strips it — the e2e Dockerfile will need it.
- **Deploy message size.** `DeployCapability` includes `component_bytes` (the full WASM binary). For the diagnostics capability this is ~1–5 MB, well within the 16 MiB codec limit. Larger capabilities may need the `/ganglion/bulk/1.0` transfer protocol (out of scope for v0.6; the control protocol is sufficient for the reference capability).
- **No peer discovery.** The operator must know the robot's peer ID and relay multiaddr. Peer discovery (via Kademlia, mDNS, or relay-side registry) is a separate concern for a future version. For v0.6, explicit `--peer` and `--relay` flags are the interface.
- **No streaming results.** `InvokeResult` returns the full output as a single `Vec<u8>`. Streaming results via `/ganglion/tool/1.0` is a future enhancement. For the diagnostics capability (which returns a single JSON bundle), single-shot is appropriate.

## Acceptance criteria

1. `gang relay` starts and prints its multiaddr (exists today).
2. `gang agent --relay <multiaddr>` starts, dials the relay, registers, and listens on `/ganglion/control/1.0`.
3. `gang deploy --peer <robot-peer-id> --relay <multiaddr> tool.wasm` sends `DeployCapability` over the relay circuit and receives a success response.
4. `gang run --peer <robot-peer-id> --relay <multiaddr> diagnostics` sends `InvokeCapability`, receives `InvokeResult` with diagnostic output.
5. `gang caps --peer <robot-peer-id> --relay <multiaddr>` sends `ListCapabilities`, receives `CapabilityList`.
6. The e2e Docker test scenario passes: relay + robot + operator, deploy diagnostics.wasm, invoke it, verify output.
7. All existing tests (188) continue to pass.
8. `gang deploy robot1 tool.wasm` (no `--peer`) continues to use the local path.
