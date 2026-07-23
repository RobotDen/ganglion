# Ganglion CLI Reference

The `gang` CLI is the primary interface for operators to manage robot identities, deploy capabilities, invoke tools, and diagnose network environments.

## Global flags

| Flag | Description |
|------|-------------|
| `--format <text\|json>` | Output format. Default: `text`. Use `json` for machine-readable output. Text-only subcommands (e.g. `identity`, `sign`, `capability scaffold`, `registry install/publish`) reject `--format json` with an error rather than silently emitting text. |
| `-v`, `-vv`, `-vvv` | Verbosity: `-v` = debug (`gang` crates), `-vv` = trace (`gang` crates), `-vvv` = trace (all crates). |
| `-q`, `--quiet` | Errors only. Conflicts with `-v`. |

`RUST_LOG`, when set, overrides the `-v`/`-q` flags for log filtering.

### Subcommand aliases

Three frequently-typed subcommands have short aliases:

| Alias | Expands to |
|-------|-----------|
| `gang id` | `gang identity` |
| `gang cap` | `gang capability` |
| `gang dx` | `gang diagnose` |

`gang --help` prints a long description of what `gang` is for and ends with a
pointer to the self-contained demo: `Run 'gang demo' for a self-contained
end-to-end demo. Docs: docs/QUICKSTART.md`.

## Status

### `gang status`

Show Ganglion version, identity status, available commands, and WIP commands.

```bash
$ gang status
Ganglion v2.0.0

Identity:   12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
Key file:   /home/user/.gang/identity.key
Registry:   2 capability(ies) registered
  dir:      /home/user/.local/share/gang/registry
Artifacts:  /home/user/.local/share/gang/artifacts
Peers:      0 registered
Config:     (not initialized — run `gang config init`)

Available commands:
  gang identity show
  gang identity generate
  gang sign
  gang agent
  ...

WIP commands (require relay connectivity):
  gang logs  [WIP]
  gang list  [WIP]
  gang connect  [WIP]
  gang transport-stats (simulated data)  [WIP]
```

Supports `--format json` for structured output.

## Identity management

### `gang identity show`

Display your peer ID and public key.

```bash
$ gang identity show
Peer ID:    12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
Public key: 4a2b3c... (hex)
```

Reads the keypair from `~/.gang/identity.key`. Generates one if it doesn't exist.

### `gang identity generate`

Generate a new Ed25519 keypair.

```bash
$ gang identity generate
Generated new identity at ~/.gang/identity.key
Peer ID: 12D3-...
```

| Flag | Description |
|------|-------------|
| `--force` | Overwrite an existing keypair without prompting. |

## Component signing

### `gang sign <wasm-path>`

Sign a WASM component and produce a manifest file.

```bash
$ gang sign my-diagnostics.wasm --name my-diagnostics \
    --component-version 0.1.0 --capabilities diagnostics,logs
Signed component: my-diagnostics.wasm
  Name:     my-diagnostics
  Version:  0.1.0
  Manifest: my-diagnostics.manifest.cbor
  Author:   12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
  Hash:     4a2b3c...
  Capabilities:
    - ganglion:diagnostics/collect@1.0
    - ganglion:logs/stream@1.0
```

| Flag | Description |
|------|-------------|
| `--key <path>` | Path to signing key. Default: `~/.gang/identity.key`. |
| `--name <name>` | Component name. Default: derived from filename. |
| `--component-version <ver>` (alias `--version`) | Component semantic version. Default: `0.1.0`. Distinct from the CLI's own `-V`/`--version`. |
| `--capabilities <c1,c2,...>` | Declared capability groups (e.g. `diagnostics,logs,ros,fs,artifacts,process,network,metrics`). |

If `--capabilities` is omitted, signing falls back to a permissive default set
(`diagnostics` + `logs "**"`) and prints a loud warning — declare capabilities
explicitly. (WIT-import auto-extraction is not yet wired.)

The manifest includes:
- Component name and version
- Author peer ID
- Blake3 hash of the `.wasm` binary
- Declared capabilities (from `--capabilities`)
- Ed25519 signature

## Robot agent

### `gang agent`

Start a robot agent. Without `--relay`, runs in local-only mode for development. With `--relay`, starts a libp2p transport, dials the relay, and serves incoming control messages on `/ganglion/control/1.0`.

```bash
# Local mode (development)
$ gang agent --data-dir /tmp/gang-agent

# Remote mode (connects to relay, accepts operator connections)
$ gang agent --data-dir /tmp/gang-agent -r /ip4/relay.example.com/tcp/4001/p2p/12D3-relay
```

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to agent config file. (Loading is not yet supported — the flag prints a warning and the agent continues with built-in dev defaults.) |
| `--data-dir <path>` | Directory for capabilities and state. Default: `/tmp/gang-agent`. |
| `--relay <multiaddr>`, `-r` | Relay multiaddr to dial for remote connectivity. |

If the relay is unreachable, the agent does **not** hang or exit: it logs a
warning, keeps serving on its listen addresses, and retries the relay dial
every 5 seconds until it succeeds.

## Capability deployment and invocation

### `gang deploy <robot> <wasm-path>`

Deploy a signed WASM component to a robot.

```bash
$ gang deploy robot-42 my-diagnostics.wasm
[log lines]
Deployed 'my-diagnostics' to robot 'robot-42'
```

The `<robot>` argument resolves through: registered name → abbreviated peer ID prefix → full peer ID → local fallback.

> **Remote dispatch is WIP.** If the target resolves to a *remote* peer, the
> command exits non-zero with a "Remote deploy ... is not yet implemented
> (ADR-020 Phase 32)" message. Only the local fallback path
> (`/tmp/gang-agent-<robot>`) currently deploys and invokes: deploy/run/caps
> run an in-process local agent over that directory (a separately started
> `gang agent` process is not consulted). The directory must exist for the
> name to resolve locally — `mkdir -p /tmp/gang-agent-<robot>` first. The same
> applies to `gang run` and `gang caps`.

| Flag | Description |
|------|-------------|
| `--manifest <path>` | Path to the manifest file. Auto-detected if adjacent to the `.wasm` file. |
| `--peer <peer-id>`, `-p` | Explicit peer ID (bypasses name/prefix resolution). |
| `--relay <multiaddr>`, `-r` | Override relay address. |

### `gang run <robot> <cap-name> [args...]`

Invoke an installed capability on a robot.

```bash
$ gang run robot-42 my-diagnostics
[log lines]
System Information:
  Hostname:  robot-42
  OS:        linux 6.18.5
  Arch:      x86_64
  CPUs:      2
  Memory:    7 GB
  Uptime:    0h 57m
  Ganglion:  v2.0.0

Network Interfaces:
  ...

Processes: 68 running
  ...
```

With `--format json`, the raw JSON result is printed instead of the
human-readable rendering.

### `gang caps <robot>`

List capabilities installed on a robot.

```bash
$ gang caps robot-42
[log lines]
Capabilities on 'robot-42':
  my-diagnostics v0.1.0 (by 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890)
    - ganglion:diagnostics/collect@1.0
```

## Log streaming

### `gang logs <robot>` [WIP]

Stream logs from a robot.

```bash
$ gang logs robot-42 --follow
```

| Flag | Description |
|------|-------------|
| `--follow` | Continuously stream new log entries (like `tail -f`). |

> Note: `[WIP]` — requires relay connectivity, which is not yet wired. The command exits non-zero with a WIP message (with `--format json` it first prints a `{"status":"unavailable",...}` object). Run `gang demo` for local testing or `gang status` for a summary of available commands.

## Diagnostics

### `gang demo`

Run a self-contained end-to-end demo. No Docker, no ROS 2, no external dependencies.

```bash
$ gang demo
=== Ganglion v2.0.0 Demo ===

Operator identity: 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
Robot agent:       12D3-b2c3d4e5f67890a1b2c3d4e5f67890a1

--- Signing diagnostics capability ---
  Component signed by 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890

--- Deploying to robot ---
  Deployed: diagnostics

--- Installed capabilities ---
  diagnostics v0.1.0 (12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890)

--- Invoking diagnostics ---
System Info:
  OS: Linux 6.1.0
  Hostname: robot-dev
  ...

--- Audit log ---
  12D3-a1b2... invoked 'diagnostics' v0.1.0 at 10:00:00 -> Success

=== Demo complete ===
Data stored at: /tmp/gang-demo
Clean up when done: rm -rf /tmp/gang-demo
```

### `gang diagnose [robot]`

Detect the network archetype and recommend transport configuration.

```bash
$ gang diagnose
Running network probes...
  internet_connectivity: PASS (DNS resolution succeeded)
  nat_status:            PASS (behind NAT — private gateway)
  multicast:             PASS (multicast-capable interfaces detected)
  outbound_ports:        PASS (TCP 53 reachable)
  dns_behavior:          PASS (TXT queries succeed)
  symmetric_nat:         FAIL (no CGNAT addresses)

Detected archetype: nat-office (confidence: 0.75)

Recommendations:
  - Configure a relay server for initial connectivity
  - DCUtR hole-punch should succeed with endpoint-independent NAT
  - Connection will upgrade to direct QUIC after hole-punch
```

If `robot` is specified, probes are run on the remote robot. If omitted, probes the local network.

### `gang transport-stats <robot>` [WIP: simulated]

Show per-transport connection statistics for a connected peer. There is no live
connection yet, so the command prints a clearly-labeled SIMULATED example. With
`--format json` the payload includes `"simulated": true`.

```bash
$ gang transport-stats robot-42
Transport statistics for: robot-42  [WIP]
(No live connection — showing SIMULATED example output.)

  Transport:       quic
  Via relay:       false
  Connect time:    145ms
  Messages:        42 sent, 38 received
  Bytes:           12.2 KB sent, 152.7 KB received
  Last RTT:        23ms
  DCUtR:           attempted=true, succeeded=true
  Uptime:          1h 0m
  Reconnections:   0
```

### `gang test-archetype <archetype>`

Launch a Docker-compose network scenario for integration testing.

```bash
$ gang test-archetype open-warehouse
```

Available archetypes: `open-warehouse`, `nat-office`, `enterprise-dmz`, `mobile-cgnat`.

Requires Docker and Docker Compose. After starting the scenario, the command
polls container state until the services are up (rather than sleeping for a
fixed interval) before running its connectivity checks.

## Content store

### `gang push <path>`

Publish a local file to the content-addressed artifact store.

```bash
$ gang push /tmp/diagnostics-bundle.tar.gz
Published: bafya1b2c3d4...
Size: 4.2 MB (5 chunks)
```

| Flag | Description |
|------|-------------|
| `--content-type <type>` | MIME type. Default: `application/octet-stream`. |

### `gang fetch <cid>`

Retrieve an artifact by its content identifier.

```bash
$ gang fetch bafya1b2c3d4... -o /tmp/bundle.tar.gz
Wrote 4404019 bytes to /tmp/bundle.tar.gz
```

The retrieved bytes are verified against the CID during retrieval. If the CID is
not in the local store, the command fails ("Remote fetch from peers is not yet
implemented"). Supports `--format json` (`{"cid","path","bytes"}`).

| Flag | Description |
|------|-------------|
| `-o`, `--output <path>` | Output path. Default: current directory. |

### `gang artifacts`

List locally-stored artifacts.

```bash
$ gang artifacts
CID                              SIZE     TYPE                    CREATED
bafya1b2c3d4...                  4.2 MB   application/octet-stream  2026-04-23T10:00:00Z
bafyb5e6f7g8...                  128 KB   application/json          2026-04-23T09:30:00Z
```

## Capability scaffolding

### `gang capability scaffold <name>`

Generate a capability project skeleton.

```bash
$ gang capability scaffold my-diagnostics --language rust
Scaffolded rust capability at ./my-diagnostics

Next steps:
  1. Implement your capability logic (WIT is in my-diagnostics/wit/ganglion.wit)
  2. Build: see docs/CAPABILITY_AUTHOR_GUIDE.md
  3. Sign: gang sign my-diagnostics.component.wasm --name my-diagnostics --version 0.1.0
```

The generated project includes a ready-to-use `wit/ganglion.wit` embedded from
the canonical in-repo WIT — you do not copy it by hand.

| Flag | Description |
|------|-------------|
| `--language <lang>` | Target language: `rust`, `cpp`, `python`, `go`. Default: `rust`. |
| `--output-dir <path>` | Output directory. Default: current directory. |

## Registry management

### `gang registry search <query>`

Search for capabilities in the local registry.

```bash
$ gang registry search diagnostics
NAME                          VERSION  DESCRIPTION
gang-capability-diagnostics   0.1.0    Basic system diagnostics
gang-capability-diag-bundle   0.1.0    Comprehensive diagnostic bundle
```

### `gang registry install <name>`

Install a capability from the registry.

```bash
$ gang registry install gang-capability-diagnostics
Installing gang-capability-diagnostics v0.1.0 ...
  Component CID: bafy...
  Manifest CID:  bafy...
  Language:       rust

Note: network fetch not yet implemented.
Use `gang fetch bafy...` to retrieve the component.
```

> Registry lookup is local; the network fetch step is a stub (WIP). Install
> resolves the entry and points you at `gang fetch` for the component bytes.

| Flag | Description |
|------|-------------|
| `--version <ver>` | Install a specific version. Default: latest. |

### `gang registry publish <wasm-path>`

Publish a signed capability to the local registry. **A signed manifest is
required** (SEC-15): the adjacent `<name>.manifest.cbor` is verified and its
authenticated contents (name, version, language, capabilities, min-version) are
the source of truth. Publishing without a signed manifest fails — sign the
component first with `gang sign`.

```bash
$ gang registry publish my-diagnostics.wasm --description "Custom diagnostics" --tags diagnostics,system
Published my-diagnostics v0.1.0 to local registry.
  Component CID: bafy...
  Registry path: /home/user/.local/share/gang/registry
```

| Flag | Description |
|------|-------------|
| `--description <text>` | Short description (overrides manifest). |
| `--tags <t1,t2,...>` | Comma-separated tags. |
| `--version <ver>` | Version to publish. Must match the signed manifest — a contradicting value is rejected. |
| `--language <lang>` | Language override: `rust`, `cpp`, `python`, `go`. |

The registry validates every entry field against the signed manifest (name,
version, capabilities, component CID). A `--version` that contradicts the
manifest fails:

```bash
$ gang registry publish my-diagnostics.wasm --version 9.9.9
Error: version mismatch: entry claims "9.9.9", signed manifest says "0.1.0"
```

### `gang registry list`

List all capabilities in the local registry.

### `gang registry info <name>`

Show detailed information for a specific capability.

## Relay server

### `gang relay`

Run a circuit relay v2 server for NAT traversal. This is the bootstrap relay
described in the design spec (`relay.gang.tafy.dev`). Robot agents behind NAT
connect to the relay so operators can reach them.

```bash
$ gang relay
Ganglion Relay Server
====================

Peer ID:      12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
Relay mode:   server
Metrics port: 9090 (not yet active)

Listen addresses:
  /ip4/0.0.0.0/tcp/4001
  /ip4/0.0.0.0/udp/4001/quic-v1

Relay is running. Press Ctrl+C to stop.
```

| Flag | Description |
|------|-------------|
| `--listen-addr <ADDR>` | Multiaddr to listen on. Can be specified multiple times. Default: TCP and QUIC on port 4001. |
| `--port <PORT>` | Port shorthand (sets both TCP and QUIC). Default: `4001`. |
| `--metrics-port <PORT>` | Metrics HTTP port (placeholder). Default: `9090`. |
| `--data-dir <PATH>` | Directory for the relay's persisted identity key. The key path is passed directly to the relay — no environment variable is set or read at relay runtime. Default: `~/.gang/identity.key`. (`GANG_KEY_PATH` is still honored by the default key-path resolution used by other commands.) |

The relay generates or loads an identity from `~/.gang/identity.key` and
prints the full relay multiaddr that clients should put in their `relay_addrs`
config. See `deploy/relay/README.md` for production deployment with Docker.

## Peer management

### `gang peer add <name> <peer-id>`

Register a known peer (robot, relay, or operator).

```bash
$ gang peer add warehouse-bot 12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890 --relay /ip4/relay.example.com/tcp/4001/p2p/12D3-relay
```

A malformed peer id is rejected with a clean error (exit code 1):

```bash
$ gang peer add badbot not-a-peer-id
Error: Invalid peer ID 'not-a-peer-id': invalid peer id: missing `12D3-` prefix: not-a-peer-id. Expected format: 12D3-<32 hex chars>
```

| Flag | Description |
|------|-------------|
| `--relay <multiaddr>`, `-r` | Relay multiaddr for reaching this peer. |
| `--role <role>` | Role: `robot-agent`, `operator`, or `relay`. Default: `robot-agent`. |

### `gang peer remove <name>`

Remove a registered peer.

### `gang peer list`

List all registered peers with their peer IDs, roles, and relay addresses.

### `gang peer show <name>`

Show details for a specific peer.

### `gang peer rename <old-name> <new-name>`

Rename a registered peer.

### `gang peer trust-reset <name>`

Clear the stored host key for a peer. The next connection will re-verify the peer's identity.

## Configuration

### `gang config show`

Show current configuration from `~/.gang/config.toml`.

### `gang config set <key> <value>`

Set a configuration value.

```bash
$ gang config set default_relay /ip4/relay.example.com/tcp/4001/p2p/12D3-relay
$ gang config set host_key_policy tofu
```

Valid keys: `default_relay`, `host_key_policy` (strict, tofu, none).

### `gang config init`

Initialize a default config file.

| Flag | Description |
|------|-------------|
| `--force` | Overwrite existing config. |

### `gang config path`

Print the config file path.

## Shell completions

### `gang completions <shell>`

Generate shell completion scripts.

```bash
gang completions bash > ~/.bash_completion.d/gang
gang completions zsh > ~/.zfunc/_gang
gang completions fish > ~/.config/fish/completions/gang.fish
```

Supported shells: bash, zsh, fish, elvish, powershell.

## Fleet management

### `gang list` [WIP]

List reachable robots in the fleet.

> Note: `[WIP]` — requires relay connectivity, which is not yet wired. The command exits non-zero with a WIP message (`--format json` prints `{"status":"unavailable",...}` first). Run `gang demo` for local testing or `gang status` for a summary of available commands.

### `gang connect <robot>` [WIP]

Establish a session with a robot via relay.

```bash
$ gang connect robot-42 --prefer-transport quic,tcp
```

| Flag | Description |
|------|-------------|
| `--prefer-transport <t1,t2>` | Preferred transport order for happy-eyeballs selection. |

> Note: `[WIP]` — requires relay connectivity, which is not yet wired. The command exits non-zero with a WIP message (`--format json` prints `{"status":"unavailable",...}` first). Run `gang demo` for local testing or `gang status` for a summary of available commands.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Any error (policy denial, trust/signature failure, I/O, WIP command, …) |
| 2 | Command-line usage error (unknown flag, missing argument — reported by clap) |

Finer-grained exit codes (distinguishing policy denials, trust-store failures,
and signature failures) are planned but not yet implemented.
