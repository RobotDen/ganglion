# Ganglion CLI Reference

The `gang` CLI is the primary interface for operators to manage robot identities, deploy capabilities, invoke tools, and diagnose network environments.

## Global flags

| Flag | Description |
|------|-------------|
| `--format <text\|json>` | Output format. Default: `text`. Use `json` for machine-readable output. |
| `-v`, `-vv`, `-vvv` | Verbosity level. `-v` = debug, `-vv` = trace (all crates). |

## Status

### `gang status`

Show Ganglion version, identity status, available commands, and WIP commands.

```bash
$ gang status
Ganglion v0.4.0

Identity:   12D3-a1b2c3d4e5f67890a1b2c3d4e5f67890
Key file:   /home/user/.gang/identity.key
Registry:   2 capability(ies) registered

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
$ gang sign my-diagnostics.wasm --name my-diagnostics --version 0.1.0
Signed: my-diagnostics.manifest.cbor
Component hash: bafy...
```

| Flag | Description |
|------|-------------|
| `--key <path>` | Path to signing key. Default: `~/.gang/identity.key`. |
| `--name <name>` | Component name. Default: derived from filename. |
| `--version <ver>` | Component version. Default: `0.1.0`. |

The manifest includes:
- Component name and version
- Author peer ID
- Blake3 hash of the `.wasm` binary
- Declared capabilities (extracted from the component)
- Ed25519 signature

## Robot agent

### `gang agent`

Start a local robot agent for development and testing.

```bash
$ gang agent --data-dir /tmp/gang-agent
Starting Ganglion agent...
Peer ID: 12D3-...
Listening for operator connections.
```

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to agent config file. |
| `--data-dir <path>` | Directory for capabilities and state. Default: `/tmp/gang-agent`. |

## Capability deployment and invocation

### `gang deploy <robot> <wasm-path>`

Deploy a signed WASM component to a robot.

```bash
$ gang deploy robot-42 my-diagnostics.wasm
Deploying my-diagnostics v0.1.0 to robot-42...
Verifying manifest signature... OK
Checking trust store... OK
Evaluating policy... OK
Deployed successfully.
```

| Flag | Description |
|------|-------------|
| `--manifest <path>` | Path to the manifest file. Auto-detected if adjacent to the `.wasm` file. |

### `gang run <robot> <cap-name> [args...]`

Invoke an installed capability on a robot.

```bash
$ gang run robot-42 diagnostics
Running diagnostics on robot-42...
{
  "system_info": { ... },
  "processes": [ ... ],
  "network_state": { ... }
}
```

### `gang caps <robot>`

List capabilities installed on a robot.

```bash
$ gang caps robot-42
NAME              VERSION  AUTHOR              INSTALLED
diagnostics       0.1.0    12D3-a1b2c3d4...   2026-04-23T10:00:00Z
param-inspect     0.1.0    12D3-a1b2c3d4...   2026-04-23T10:05:00Z
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

> Note: Requires relay connectivity (not yet available in v0.4). Run `gang demo` for local testing or `gang status` for a summary of available commands.

## Diagnostics

### `gang demo`

Run a self-contained end-to-end demo. No Docker, no ROS 2, no external dependencies.

```bash
$ gang demo
=== Ganglion Demo ===
Generating identity...
Starting local agent...
Deploying diagnostics capability...
Invoking diagnostics...

System Info:
  OS: Linux 6.1.0
  Hostname: robot-dev
  Uptime: 34521s
  ...
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

### `gang transport-stats <robot>`

Show per-transport connection statistics for a connected peer.

```bash
$ gang transport-stats robot-42
Transport: quic
  Via relay: false
  RTT: 12ms
  Messages sent: 142
  Messages received: 138
  Bytes sent: 48.2 KB
  Bytes received: 1.2 MB
  DCUtR: upgraded
  Uptime: 3421s
```

### `gang test-archetype <archetype>`

Launch a Docker-compose network scenario for integration testing.

```bash
$ gang test-archetype open-warehouse
```

Available archetypes: `open-warehouse`, `nat-office`, `enterprise-dmz`, `mobile-cgnat`.

Requires Docker and Docker Compose.

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
Retrieved: 4.2 MB
Verified: hash matches CID
```

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
Created my-diagnostics/
  ├── Cargo.toml
  ├── src/lib.rs
  ├── wit/
  │   └── ganglion.wit
  └── Makefile
```

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
Installing gang-capability-diagnostics v0.1.0...
Installed to ~/.gang/capabilities/
```

| Flag | Description |
|------|-------------|
| `--version <ver>` | Install a specific version. Default: latest. |

### `gang registry publish <wasm-path>`

Publish a signed capability to the local registry.

```bash
$ gang registry publish my-diagnostics.wasm --description "Custom diagnostics" --tags diagnostics,system
Published: my-diagnostics v0.1.0
```

| Flag | Description |
|------|-------------|
| `--description <text>` | Short description. |
| `--tags <t1,t2,...>` | Comma-separated tags. |

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

The relay generates or loads an identity from `~/.gang/identity.key` and
prints the full relay multiaddr that clients should put in their `relay_addrs`
config. See `deploy/relay/README.md` for production deployment with Docker.

## Fleet management

### `gang list` [WIP]

List reachable robots in the fleet.

> Note: Requires relay connectivity (not yet available in v0.4). Run `gang demo` for local testing or `gang status` for a summary of available commands.

### `gang connect <robot>` [WIP]

Establish a session with a robot via relay.

```bash
$ gang connect robot-42 --prefer-transport quic,tcp
```

| Flag | Description |
|------|-------------|
| `--prefer-transport <t1,t2>` | Preferred transport order for happy-eyeballs selection. |

> Note: Requires relay connectivity (not yet available in v0.4). Run `gang demo` for local testing or `gang status` for a summary of available commands.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Policy denied |
| 3 | Trust store verification failed |
| 4 | Component signature invalid |
