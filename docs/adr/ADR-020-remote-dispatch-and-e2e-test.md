# ADR-020: Remote dispatch via control protocol and end-to-end validation

**Status:** Accepted; partially implemented
**Date:** 2026-04-24

> **Implementation status (as of v2.0.0).** The operator-experience and
> supporting pieces landed: peer registry CLI (§4), operator config file (§5),
> SSH-style TOFU identity verification (§6), shell completions (§8), the
> reference WASM component build (§9), and target resolution (name → prefix →
> peer id → local fallback) in §3. **Not yet implemented:** the robot agent
> serve loop (§1) and relay-mediated operator remote dispatch (§2–§3) — a
> resolved *remote* target exits with a "not yet implemented (ADR-020 Phase 32)"
> message and only the local fallback executes. The e2e scenario (§7) is
> therefore a **connectivity smoke test**, not the full deploy/invoke round-trip
> described below. The design below is retained as the target design.

## Context

Ganglion's architecture defines three roles (relay, robot agent, operator) communicating over libp2p via `/ganglion/control/1.0`. The infrastructure for this exists:

- **Message types:** `ControlMessage` enum with `DeployCapability`, `InvokeCapability`, `InvokeResult`, `ListCapabilities`, `Error`, `Presence` variants, CBOR-encoded with varint-prefixed framing (`gang-core::message`).
- **Transport:** `Libp2pTransportAdapter` with `request_response::Behaviour<GanglionCodec>`, 16 MiB length-prefixed codec, `dial()`, `listen()`, `handle_rpc_message()` dispatch.
- **Protocol constants:** `PROTOCOL_CONTROL`, `PROTOCOL_TOOL`, `PROTOCOL_BULK` defined in `gang-core::protocol`.
- **Relay server:** `gang relay` starts a circuit relay v2 server and prints its multiaddr.
- **Peer registry:** `PeerRegistry` in `gang-core::identity` maps human-readable names to peer IDs, roles, and relay addresses. Fully implemented with CRUD, persist/reload. Stored at `~/.gang/peers.json`. Not yet wired to the CLI.
- **Trust store:** `TrustStore` in `gang-core::manifest` stores peer IDs with their Ed25519 public keys. Used for capability signature verification. Stored at `~/.gang/trusted_peers.json`.

However, two critical pieces are missing:

1. **Robot side:** `gang agent` creates a `RobotAgent` and waits for Ctrl+C. It never starts a libp2p transport, never dials a relay, and never calls `transport.listen()` to register a handler for incoming `ControlMessage` requests. There is no `serve()` method.

2. **Operator side:** `gang deploy` and `gang run` construct a *local* `RobotAgent` at `/tmp/gang-agent-{robot}` and call `deploy_capability()` / `invoke_capability()` directly in-process. They never dial a remote peer over libp2p.

The design spec (§4.2, §4.5) states that v0.1 should demonstrate deploy and invoke "across a simulated hostile network." The local-only path was acceptable for proving the broker architecture, but the project's thesis — hostile-network reachability — requires the relay-mediated remote path to be real.

Additionally, the developer and operator experience needs attention:

- Peer IDs (`12D3-a1b2c3d4...`) are 37-character hex strings — not memorable, not typeable. The existing `PeerRegistry` supports name→ID mapping but the CLI doesn't expose it.
- There is no identity verification on first connect. An operator connecting to a robot for the first time has no way to verify they're talking to the right machine, and no warning if a known peer's identity changes (the SSH `known_hosts` problem).
- Specifying `--peer` and `--relay` on every command is verbose for routine operations against known robots.

## Decision

Implement remote dispatch in v0.6 across eight work areas:

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

- Accept `--relay <multiaddr>` / `-r <multiaddr>` flag (required for remote mode, optional for local-only dev mode).
- Create `Libp2pTransportAdapter` with `relay_server: false` and the agent's identity.
- Dial the relay multiaddr (establishing the circuit reservation).
- Call `agent.serve(&transport)` to begin listening on the control protocol.
- Print the robot's peer ID and relay circuit multiaddr so the operator knows how to reach it.
- Continue running the transport event loop until Ctrl+C.

Without `--relay`, the agent behaves as today (local mode, no transport) for backward compatibility with `gang demo` and local testing.

### 3. Operator remote dispatch (`gang-cli`)

Modify `gang deploy`, `gang run`, and `gang caps` to support a unified robot target syntax:

**Target resolution order.** The `<robot>` positional argument resolves through:

1. **Registered name** — looked up in `PeerRegistry` (`~/.gang/peers.json`). If found, use the stored peer ID and relay address. Example: `gang run warehouse-bot diagnostics`.
2. **Abbreviated peer ID** — if the argument starts with `12D3-` and is shorter than 37 characters, search the registry for a unique prefix match (Docker-style). Example: `gang run 12D3-a1b2 diagnostics`. Ambiguous prefixes produce an error listing matches.
3. **Full peer ID** — 37-character `12D3-...` string. Connects via the default relay (from config) or the `--relay`/`-r` override.
4. **Local fallback** — if none of the above match and `/tmp/gang-agent-{robot}` exists, use the local path (backward compatible with `gang demo`).

**Flags:**

- `--relay <multiaddr>` / `-r` — override the relay address. If omitted, use the relay stored in the peer registry entry, or fall back to the default relay from config.
- `--peer <peer-id>` / `-p` — explicit peer ID (bypasses name/prefix resolution). Retained for scripting and when the peer isn't registered.

**Connection flow:**

1. Resolve target to peer ID + relay address.
2. Create `Libp2pTransportAdapter` with the operator's identity.
3. Dial the relay, then request a circuit to the robot's peer ID.
4. **Verify identity** (see §6 below).
5. Serialize the appropriate `ControlMessage` via `encode_message`.
6. Send via `transport.send_request()` and await the response.
7. Deserialize the response as `ControlMessage`.
8. Print the result in the requested format (text or JSON).
9. Shut down the transport after the operation completes.

**Examples:**

```bash
# By registered name (most common)
gang deploy warehouse-bot diagnostics.wasm
gang run warehouse-bot diagnostics
gang caps warehouse-bot

# By abbreviated peer ID (Docker-style)
gang run 12D3-a1b2 diagnostics

# By full peer ID with explicit relay
gang run -p 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890 -r /ip4/relay.example.com/tcp/4001/p2p/12D3-relay diagnostics

# Local mode (backward compatible, no network)
gang run robot1 diagnostics   # falls back to /tmp/gang-agent-robot1
```

### 4. Peer registry CLI (`gang-cli`)

Wire the existing `PeerRegistry` to new CLI commands:

```
gang peer add <name> <peer-id> [--relay <multiaddr>] [--role robot-agent|operator|relay]
gang peer remove <name>
gang peer list
gang peer show <name>
gang peer rename <old> <new>
```

`gang peer add` is the primary registration mechanism. When `gang agent --relay` starts, it prints a one-liner the operator can paste:

```
$ gang agent --relay /ip4/relay.example.com/tcp/4001/p2p/12D3-abc123
Robot agent started:
  Peer ID:  12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
  Relay:    /ip4/relay.example.com/tcp/4001/p2p/12D3-abc123

  Register on operator machine:
    gang peer add my-robot 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890 --relay /ip4/relay.example.com/tcp/4001/p2p/12D3-abc123
```

`gang peer list` shows all registered peers with their names, abbreviated IDs, roles, and relay addresses:

```
NAME            PEER ID         ROLE          RELAY
warehouse-bot   12D3-a1b2c3d4   robot-agent   /ip4/relay.example.com/tcp/4001/p2p/12D3-abc123
lab-arm         12D3-e5f67890   robot-agent   /ip4/10.0.0.5/tcp/4001/p2p/12D3-def456
staging-relay   12D3-abc12345   relay         (self)
```

### 5. Operator config file (`gang-cli`)

Add `~/.gang/config.toml` for defaults that eliminate repetitive flags:

```toml
# Default relay for all operations when --relay is not specified
# and the peer registry entry has no relay_addrs.
default_relay = "/ip4/relay.gang.tafy.dev/tcp/4001/p2p/12D3-..."

# Identity verification policy (see §6)
# Options: "strict" (default), "tofu", "none"
host_key_policy = "strict"
```

Precedence: CLI flag > peer registry entry > config file > hardcoded default.

### 6. Identity verification — SSH-style host key checking (`gang-core`)

Prevent peer ID spoofing by verifying the remote peer's public key on every connection:

**First connect (Trust On First Use — TOFU):**

When connecting to a peer ID for the first time (no public key stored in the trust store), the operator sees:

```
The authenticity of robot '12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890' can't be established.
Ed25519 key fingerprint is SHA256:xB3kZ9...
Are you sure you want to continue connecting (yes/no)?
```

On `yes`, the public key is recorded in `~/.gang/trusted_peers.json` alongside the peer ID. Subsequent connections verify silently.

**Key change (identity mismatch):**

If a known peer presents a different public key:

```
@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!    @
@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!
The Ed25519 host key for robot 'warehouse-bot' (12D3-a1b2c3d4...) has changed.
Fingerprint for the new key: SHA256:yC4lA0...
Add correct host key in ~/.gang/trusted_peers.json to get rid of this message.
Offending key stored at index 3.
Robot key verification failed.
```

Connection is refused. The operator must manually remove the old key (`gang peer trust-reset <name>`) before reconnecting. This is the SSH `StrictHostKeyChecking` model.

**Policy options** (in `config.toml`):

- `strict` (default) — TOFU on first connect, hard fail on mismatch.
- `tofu` — always accept new keys without prompting, hard fail on mismatch.
- `none` — no verification (for development/testing only). Prints a warning.

**Implementation:** The libp2p Noise handshake already exchanges public keys as part of connection establishment. After the handshake completes, extract the remote public key from the Noise session and compare against the trust store entry for that peer ID. This is a check on data already available — no additional round trip.

### 7. End-to-end test scenario

A new Docker Compose scenario (`test-harness/e2e-dispatch/`) that validates the full flow:

**Topology:** Three containers on a flat bridge network (simplest case — proving protocol correctness, not network hostility, which the existing 4 scenarios cover).

- `relay` — runs `gang relay --port 4001`
- `robot` — runs `gang agent -r /ip4/<relay-ip>/tcp/4001/p2p/<relay-peer-id> --data-dir /data`
- `operator` — runs a test script that:
  1. Waits for the relay and robot to be ready (poll relay's TCP port, then check robot's presence).
  2. Registers the robot: `gang peer add test-robot <robot-peer-id> -r <relay-multiaddr>`.
  3. Deploys: `gang deploy test-robot /test-data/diagnostics.wasm`.
  4. Invokes: `gang run test-robot diagnostics`.
  5. Asserts: exit code 0, output contains expected diagnostic fields.
  6. Lists capabilities: `gang caps test-robot`.
  7. Asserts: output lists `diagnostics`.
  8. Lists peers: `gang peer list`.
  9. Asserts: output shows `test-robot` with correct peer ID.

**Test payload:** `gang-capability-diagnostics` compiled to a WASM component (`.wasm`) and signed with a test keypair. The build step compiles the capability crate to `wasm32-wasip2`, runs `wasm-tools component new` to produce a component, and signs it with `gang sign`.

**Test runner:** `test-harness/e2e-dispatch/run-test.sh` builds the base image, compiles the test WASM component, starts the scenario, runs the operator script, checks results, tears down.

### 8. Shell completions (`gang-cli`)

Add `gang completions <shell>` to generate shell completion scripts for popular shells:

```
gang completions bash    > /etc/bash_completion.d/gang
gang completions zsh     > ~/.zfunc/_gang
gang completions fish    > ~/.config/fish/completions/gang.fish
gang completions elvish  > ~/.config/elvish/lib/gang.elv
gang completions powershell > gang.ps1
```

**Implementation:** clap provides `clap_complete::generate()` which produces completions from the existing `Cli` struct. This is a one-function addition:

1. Add `clap_complete` to `gang-cli` dependencies.
2. Add a `Completions { shell: clap_complete::Shell }` variant to `Commands`.
3. In the match arm, call `clap_complete::generate(shell, &mut Cli::command(), "gang", &mut io::stdout())`.

**Dynamic completions for peer names:** clap_complete supports custom value hints. Register a completer for `<robot>` positional arguments that reads `~/.gang/peers.json` and returns registered peer names. This gives tab-completion for `gang run <TAB>` → `warehouse-bot`, `lab-arm`, etc.

**Dynamic completions for capability names:** For commands like `gang run <robot> <TAB>`, complete capability names by reading the local cache of installed capabilities (from the last `gang caps` result or the local agent's capabilities directory). This is best-effort — network queries for live completion are too slow.

### 9. Reference WASM component build

Add a build target for `gang-capability-diagnostics` that produces a signed `.wasm` component:

- `Makefile` or `build.rs` in `crates/gang-capability-diagnostics/` that:
  1. `cargo build --target wasm32-wasip2 -p gang-capability-diagnostics`
  2. `wasm-tools component new target/wasm32-wasip2/release/gang_capability_diagnostics.wasm -o diagnostics.wasm`
  3. `gang sign diagnostics.wasm --name diagnostics --key <test-key>`
- This is the test payload for the e2e scenario and the reference artifact for capability authors.

## Future: TUI mode

A terminal UI for Ganglion is planned but out of scope for v0.6. The v0.6 CLI design is structured to enable it:

- `gang peer list` returns structured data (JSON with `--json` flag) that a TUI can render as a navigable table.
- The `PeerRegistry` is the data model a TUI would display and mutate.
- The `ControlMessage` protocol is request-response, which maps naturally to TUI actions (select robot → deploy/run/caps → show result).
- A TUI crate (`gang-tui`) would depend on `gang-core` and `gang-libp2p`, use `ratatui` or similar, and present: peer list, capability list per robot, deploy/invoke actions, log streaming, and connection status. The v0.6 serve loop and remote dispatch are prerequisites — without them there's nothing for a TUI to drive.

## Consequences

### Positive

- **Proves the thesis.** The project's central claim — hostile-network reachability — is validated with an actual relay-mediated deploy and invoke, not just a local in-process simulation.
- **Uses existing infrastructure.** `ControlMessage`, `GanglionCodec`, `handle_rpc_message()`, `encode_message`/`decode_message`, `TransportAdapter`, `PeerRegistry`, `TrustStore` are all implemented and tested. The new code connects them; it doesn't replace them.
- **Operator-friendly CLI.** Named peers, abbreviated IDs, config-file defaults, and short flags (`-p`, `-r`) eliminate the verbose flag ceremony. An operator who has registered a robot types `gang run warehouse-bot diagnostics`, not `gang run --peer 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890 --relay /ip4/relay.example.com/tcp/4001/p2p/12D3-abc123 diagnostics`.
- **Identity security.** SSH-style TOFU prevents silent peer ID spoofing. The warning on key change is immediately recognizable to anyone who has used SSH.
- **Backward compatible.** `gang deploy robot1 tool.wasm` (no `--peer` flag) continues to work locally. The local path remains the fast-iteration development workflow.
- **Validates the WASM component pipeline.** Building a real `.wasm` component from `gang-capability-diagnostics` proves the authoring pipeline works end-to-end.
- **CI-testable.** The flat-network e2e scenario runs on GitHub Actions ubuntu-latest without privilege escalation.
- **TUI-ready.** The peer registry, config file, and structured command outputs form the data layer a future TUI needs.
- **Shell completions.** Tab-completion for commands, subcommands, flags, and dynamic peer names reduces typing and discoverability friction. Works across bash, zsh, fish, elvish, and PowerShell.

### Negative

- **WASM component compilation requires `wasm32-wasip2` target.** This target must be installed (`rustup target add wasm32-wasip2`) and `wasm-tools` must be available. CI must install these.
- **Deploy message size.** `DeployCapability` includes `component_bytes` (the full WASM binary). For the diagnostics capability this is ~1–5 MB, well within the 16 MiB codec limit. Larger capabilities may need the `/ganglion/bulk/1.0` transfer protocol (out of scope for v0.6).
- **TOFU is not zero-trust.** First-connect trust is a pragmatic compromise. Operators in high-security environments should pre-provision trust store entries rather than relying on TOFU. The `strict` default and the loud key-change warning mitigate the risk.
- **No streaming results.** `InvokeResult` returns the full output as a single `Vec<u8>`. Streaming results via `/ganglion/tool/1.0` is a future enhancement.

## Acceptance criteria

1. `gang relay` starts and prints its multiaddr (exists today).
2. `gang agent -r <multiaddr>` starts, dials the relay, registers, and listens on `/ganglion/control/1.0`.
3. `gang peer add my-robot <peer-id> -r <relay-addr>` registers the robot in `~/.gang/peers.json`.
4. `gang deploy my-robot tool.wasm` resolves the name, sends `DeployCapability` over the relay circuit, receives success.
5. `gang run my-robot diagnostics` sends `InvokeCapability`, receives `InvokeResult` with diagnostic output.
6. `gang run 12D3-a1b2 diagnostics` resolves by abbreviated peer ID prefix.
7. `gang caps my-robot` sends `ListCapabilities`, receives `CapabilityList`.
8. First connection to an unknown peer prompts for TOFU confirmation.
9. Connection to a peer whose public key has changed is refused with a loud warning.
10. `gang peer list` shows registered peers with names, abbreviated IDs, roles, and relay addresses.
11. `~/.gang/config.toml` default_relay is used when no `-r` flag and no registry entry relay.
12. The e2e Docker test scenario passes: relay + robot + operator, register peer, deploy, invoke, verify output.
13. All existing tests (188) continue to pass.
14. `gang deploy robot1 tool.wasm` (no registered peer, no `--peer`) continues to use the local path.
15. `gang completions bash` produces valid bash completion script.
16. Tab-completing `gang run <TAB>` suggests registered peer names.
