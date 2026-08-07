# Ganglion CLI Reference

The `gang` CLI is the primary interface for operators to manage robot identities, deploy capabilities, invoke tools, and diagnose network environments.

## Global flags

| Flag | Description |
|------|-------------|
| `--format <text\|json>` | Output format. Default: `text`. Use `json` for machine-readable output. Text-only subcommands (e.g. `identity`, `sign`, `capability scaffold`, `registry install/publish`) reject `--format json` with an error rather than silently emitting text. |
| `-v`, `-vv`, `-vvv` | Verbosity: `-v` = debug (`gang` crates), `-vv` = trace (`gang` crates), `-vvv` = trace (all crates). |
| `-q`, `--quiet` | Errors only. Conflicts with `-v`. |
| `--data-dir <path>` | Point the whole CLI at a self-contained fleet directory instead of `~/.gang` (identity, peer registry, config, trust store). This is the directory `gang up` stands a fleet up in; pass the same value here to drive it: `gang --data-dir <dir> deploy up-robot …`. |

`RUST_LOG`, when set, overrides the `-v`/`-q` flags for log filtering.

### Subcommand aliases

Three frequently-typed subcommands have short aliases:

| Alias | Expands to |
|-------|-----------|
| `gang id` | `gang identity` |
| `gang cap` | `gang capability` |
| `gang dx` | `gang diagnose` |
| `gang fleet` | `gang up` |

`gang --help` prints a long description of what `gang` is for and ends with a
pointer to the self-contained demo: `Run 'gang demo' for a self-contained
end-to-end demo. Docs: docs/QUICKSTART.md`.

## First-run setup

### `gang init`

Guided first-run setup — take a fresh install to *configured* in one command.
It collapses the read-the-architecture-docs phase into a single step:

1. **Archetype detection** — runs the same network probes as `gang diagnose`
   and prints the detected archetype plus its transport implication.
2. **Identity** — generates the operator identity if none exists (never
   clobbers an existing key without `--force`).
3. **Policy + config** — writes a genuinely default-deny `policy.toml` (no
   capability group permitted; commented example rules to uncomment) plus an
   operator `config.toml` (defaults, incl. `host_key_policy = strict`).
4. **Next steps** — prints a short, correctly-ordered panel of real commands
   tailored to the detected archetype.

Interactive on a TTY (a couple of skippable `[Y/n]` prompts with safe
defaults); fully non-interactive when stdin is a pipe/CI or `--yes` is given.
Re-running is idempotent: existing files are reported and kept, never
overwritten without `--force`.

**Flags:**

| Flag | Description |
|------|-------------|
| `--data-dir <path>` | Global flag: point setup at this directory instead of `~/.gang` (identity, policy, config land here). |
| `--force` | Overwrite an existing identity, policy, or config. Regenerating the identity rotates your peer id. |
| `-y`, `--yes` (alias `--non-interactive`) | Skip prompts and use safe defaults. Implied when stdin is not a TTY. |
| `--json` | Emit the resulting setup (identity id, archetype, paths, next commands) as a single JSON object instead of the text panel. |

```console
$ gang init --yes
=== gang init — configuring Ganglion ===

Data dir: /home/you/.gang

[1/4] Network archetype
  Detected:  regulated-facility (80% confidence)
  Transport: No network connectivity detected — use offline signed bundles

[2/4] Operator identity
  Generated: 12D3-56e26108b7dd14c146597c33e5ffa839
  Key file:  /home/you/.gang/identity.key

[3/4] Policy + config
  Wrote default-deny policy: /home/you/.gang/policy.toml
  Wrote operator config:     /home/you/.gang/config.toml  (host_key_policy = strict)

[4/4] You're configured. What to run next

  # Try a live local fleet on loopback right now:
  gang up

  # For a real deployment (regulated-facility):
  #   Air-gapped: skip the relay. Sign capabilities here with `gang sign` and move the signed bundle to the robot over approved media.
  gang sign <component.wasm> --capabilities <groups> # on this workstation
  # transfer <component>.wasm + .manifest.cbor over approved media
  gang deploy <name> <signed.wasm>       # on the robot host

  # Enrol a robot (gang pair is coming; today use peer add):
  gang peer add <name> <robot-libp2p-id> --relay <relay-multiaddr>

Run `gang status` to review your configuration.
```

The next-steps panel adapts to the archetype: on a networked archetype it prints
a `gang relay` / `gang agent` / `gang peer add` / `gang deploy` sequence (with the
relay pinned to TCP 443 for `enterprise-dmz`); on `regulated-facility` it prints
the offline `gang sign` + transfer path shown above.

Re-running without `--force` is non-destructive:

```console
$ gang init --yes
...
[2/4] Operator identity
  Already present: 12D3-56e26108b7dd14c146597c33e5ffa839
  Key file:        /home/you/.gang/identity.key
  (use --force to regenerate — this rotates your peer id)

[3/4] Policy + config
  Policy exists, kept:   /home/you/.gang/policy.toml
  Config exists, kept:   /home/you/.gang/config.toml
...
```

With `--json`:

```console
$ gang init --json
{
  "archetype": {
    "confidence": 0.8,
    "name": "regulated-facility",
    "transport": "No network connectivity detected — use offline signed bundles"
  },
  "config_created": true,
  "config_path": "/home/you/.gang/config.toml",
  "data_dir": "/home/you/.gang",
  "identity": {
    "created": true,
    "existed": false,
    "id": "12D3-674dcd7773b8dc307afa077bc49efd5d",
    "key_path": "/home/you/.gang/identity.key"
  },
  "next_commands": [
    "gang up",
    "gang sign <component.wasm> --capabilities <groups>",
    "gang deploy <name> <signed.wasm>"
  ],
  "policy_created": true,
  "policy_path": "/home/you/.gang/policy.toml",
  "status": "configured"
}
```

## Robot enrollment

One-line robot onboarding — the operator runs one command, the robot runs one
copy-paste line, and the robot appears in `gang peer list` ready to drive. See
[ADR-021](adr/ADR-021-pairing-token-enrollment.md) for the trust model.

### `gang pair`

Run on the **operator** machine. Mints a short-lived, single-use *pairing token*
bound to the relay and this operator's identity, prints ONE line to run on the
robot, then waits: when the robot dials out and enrolls, the operator records it —
under the identity libp2p authenticated on the wire, never a self-report — so it
appears in `gang peer list` ready for `gang deploy`/`gang run`.

**Flags:**

| Flag | Description |
|------|-------------|
| `--relay <multiaddr>` (`-r`) | Relay the robot should dial (default: `default_relay` from config). The dialable form printed by `gang relay`/`gang up`. |
| `--name <name>` | Name to register the robot under (default: `robot-<short-id>`). The robot may also request a name via `gang join --name`. |
| `--expires <duration>` | Token lifetime: `90s`, `15m`, `1h`, or a bare number of seconds (default: `15m`). |
| `--qr` | Also render the robot line as a QR code (currently prints a note that QR is a follow-up; see ADR-021). |
| `--timeout <secs>` | Give up waiting for the robot after this many seconds (default: `300`). |
| `--json` | Emit the token/relay/operator facts as JSON, then wait as usual. |

```console
$ gang pair --relay /ip4/127.0.0.1/tcp/45633/p2p/12D3KooWM3tJywVGi7MjE4g6RWEGhxK6C2iSyeomoCeyN8RVEoRU --name field-01
=== gang pair — enroll a robot in one line ===

Relay:    /ip4/127.0.0.1/tcp/45633/p2p/12D3KooWM3tJywVGi7MjE4g6RWEGhxK6C2iSyeomoCeyN8RVEoRU
Operator: 12D3-74aded42b4ed2b3e88d1a4cedd8ec501
Expires:  2026-08-06T03:50:13.922+00:00

Run this ONE line on the robot:

    gang join gang1_pWd2ZXJzaW9uAWpyZWxheV9hZGRyeFEvaXA0LzEyNy4wLjAuMS90Y3AvNDU2MzMvcDJwLzEyRDNLb29XTTN0Snl3VkdpN01qRTRnNlJXRUdoeEs2QzJpU3llb21vQ2V5TjhSVkVvUlVyb3BlcmF0b3JfbGlicDJwX2lkeDQxMkQzS29vV0FzTjIz...

Waiting up to 300s for the robot to dial out and enroll… (Ctrl-C to cancel)

  ✔ paired: field-01  (12D3-d0d9d07c0b8480d876a10cf1e05750c2)

The robot is now in your fleet. Drive it:
  gang deploy field-01 <signed.wasm>
  gang run field-01 <capability>
  gang peer list
```

> The token line is long — it self-describes the relay and operator so the robot
> needs no other configuration. QR output is a documented follow-up (ADR-021):
> `--qr` prints the copy-paste line rather than adding an unapproved dependency.

### `gang join <token>`

Run on the **robot** — the ONE line printed by `gang pair`. Decodes the token,
loads or generates this robot's identity, dials out to the relay, reserves a
circuit, and enrolls with the operator the token names (whose identity libp2p
authenticates end-to-end). Then it keeps serving as the agent so the operator can
deploy immediately — exactly like `gang agent`.

**Flags:**

| Flag | Description |
|------|-------------|
| `--name <name>` | Name to request from the operator (default: `robot-<short-id>`). |
| `--once` | Enroll and exit instead of staying online as the agent. |
| `--timeout <secs>` | Overall budget for the enrollment exchange (default: `60`). |
| `--json` | Emit the enrollment result as JSON. |

```console
$ gang join gang1_pWd2ZXJzaW9uAWpyZWxheV9hZGRy… --name field-01
Joining fleet via /ip4/127.0.0.1/tcp/45633/p2p/12D3KooWM3tJywVGi7MjE4g6RWEGhxK6C2iSyeomoCeyN8RVEoRU…

  ✔ joined: registered with operator 12D3-74aded42b4ed2b3e88d1a4cedd8ec501 as 'field-01'
    this robot: 12D3-d0d9d07c0b8480d876a10cf1e05750c2

Serving on the relay circuit. Press Ctrl-C to stop.
```

Back on the operator, the robot is now a normal fleet member:

```console
$ gang peer list
NAME             PEER ID          DIAL ID          ROLE           RELAY
field-01         12D3-d0d9d07c0b8 12D3KooWG32VJCM1 robot-agent /ip4/127.0.0.1/tcp/45633/p2p/12D3KooWM3tJywVGi7MjE4g6RWEGhxK6C2iSyeomoCeyN8RVEoRU

$ gang deploy field-01 diagnostics.wasm
Deployed 'diagnostics' to robot 'field-01' (via relay)

$ gang run field-01 diagnostics
System Information:
  Hostname:  vm
  ...
```

The token is single-use and expiring: a reused or expired token is rejected, and
the operator only ever records the id libp2p authenticated on the wire — a robot
cannot enroll as an identity whose key it does not hold. If you prefer manual
registration (air-gapped or scripted), `gang peer add` remains the fallback.

## Status

### `gang status`

Show Ganglion version, identity status, and available commands.

```bash
$ gang status
Ganglion v2.1.0

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
  gang logs
  gang connect
  gang tui
  gang list
  gang transport-stats
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
$ gang agent --data-dir /tmp/gang-agent \
    -r /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

The `/p2p/` component of the relay multiaddr must be the relay's
**libp2p-format** peer ID (base58, `12D3KooW...`) — the Ganglion-native
`12D3-<hex>` form does not parse in a multiaddr. `gang relay` prints the
correct value as `Peer ID (libp2p/dial)` (and ready-to-paste client
multiaddrs) at startup.

In relay mode the agent requests a **circuit reservation** on the relay (the
reservation is what makes the robot reachable through it), prints its own
dialable id as `Peer ID (libp2p/dial): 12D3KooW…`, and prints the exact
`gang peer add` line to run on the operator machine.

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

Deploy a signed WASM component to a robot — remotely over the relay circuit,
or locally against an in-process agent.

The `<robot>` argument resolves through: registered name → abbreviated peer ID
prefix → full peer ID (gang `12D3-<hex>` or dialable libp2p `12D3KooW…`) →
local fallback.

**Remote targets** (a registered peer with a stored libp2p id and relay
address): the CLI builds a transport from your operator identity, dials the
relay, dials the robot via `<relay>/p2p-circuit/p2p/<robot-libp2p-id>`,
verifies the robot's host key (SSH-style TOFU — see `host_key_policy` under
`gang config`), and sends the signed bundle on `/ganglion/control/1.0` with a
fresh nonce + timestamp (the robot rejects stale or replayed requests):

```bash
$ gang deploy robot-a my-tool.wasm
Deployed 'my-tool' to robot 'robot-a' (via relay)
```

A remote failure exits non-zero with the robot's actual error, e.g.:

```
Error: timed out after 60s: robot 'robot-a' not reachable via relay
/ip4/203.0.113.10/tcp/4001/p2p/12D3KooW... (is the agent running, and did it
connect to that relay?)
```

A peer registered with only a legacy gang id cannot be dialed; the command
tells you to re-add it with the libp2p id the agent prints at startup.

**Local fallback** (`/tmp/gang-agent-<robot>` exists): deploy/run/caps run an
in-process local agent over that directory (a separately started `gang agent`
process is not consulted). The directory must exist for the name to resolve
locally — `mkdir -p /tmp/gang-agent-<robot>` first.

```bash
$ gang deploy robot-42 my-tool.wasm
[log lines]
Deployed 'my-tool' to robot 'robot-42'
```

| Flag | Description |
|------|-------------|
| `--manifest <path>` | Path to the manifest file. Auto-detected if adjacent to the `.wasm` file. |
| `--peer <peer-id>`, `-p` | Explicit peer ID (bypasses name/prefix resolution; accepts either id form). |
| `--relay <multiaddr>`, `-r` | Override relay address. |
| `--timeout <secs>` | Overall remote-dispatch timeout. Default: 60. |

### `gang run <robot> <cap-name> [args...]`

Invoke an installed capability on a robot (remote dispatch and local fallback
work exactly as for `gang deploy`; remote default timeout is 30 s).

```bash
$ gang run robot-42 my-tool
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
human-readable rendering. Non-JSON capability output is printed as text. A
non-success invocation status from the robot (failure, policy denial, trap,
timeout) exits non-zero.

| Flag | Description |
|------|-------------|
| `--peer <peer-id>`, `-p` | Explicit peer ID (bypasses name/prefix resolution; accepts either id form). |
| `--relay <multiaddr>`, `-r` | Override relay address. |
| `--timeout <secs>` | Overall remote-dispatch timeout. Default: 30. |

### `gang caps <robot>`

List capabilities installed on a robot (remote dispatch and local fallback as
above; remote default timeout is 30 s, `--timeout` to override).

```bash
$ gang caps robot-a
Capabilities on 'robot-a':
  my-tool v0.1.0 (by 12D3-68c62c79b89c56c575df0845f26b6fae)
    - ganglion:diagnostics/collect@1.0
```

## Log streaming

### `gang logs <robot>`

Subscribe to a robot's event feed over the relay circuit and print its
`AuditAppended` and `PolicyDecision` events. Without `--follow`, prints the
recent context from the robot's retained window and exits; with `--follow`,
tails live (Ctrl-C to stop). Honest non-zero exit if the robot is unreachable or
refuses the subscription.

The feed transport is selectable with `--events-transport <auto|push|poll>`
(default `auto`, ADR-024):

- `auto` — prefer the genuine server-push substream (events print the instant
  the robot emits them); fall back automatically to the request-response poll
  when push is unavailable (older/alpha-free agent, protocol-not-supported) or
  if a push stream drops mid-session.
- `push` — force push; error clearly if the stream cannot be opened.
- `poll` — force the request-response poll on a ~1.5 s cadence (configurable via
  the operator config's `events_poll_interval_ms`).

`gang logs`/`connect`/`tui` share this flag; the operator config field
`events_transport` sets the default. `logs` prints a `--- feed: push ---` /
`--- feed: poll (1.5s) ---` line so the active transport is visible.

```bash
$ gang logs up-robot
2026-08-06T04:42:35Z  policy ALLOW  ganglion:diagnostics/collect  by 12D3-715cfb78…  (capabilities permitted by policy)
2026-08-06T04:42:36Z  audit  diagnostics v0.1.0  by 12D3-715cfb78…  -> success  caps=[ganglion:diagnostics/collect@1.0]

$ gang --format json logs up-robot
{"type":"policy_decision","seq":3,"ts":"2026-08-06T04:42:35.666895790Z","operator_peer":"12D3-715cfb78…","capability_group":"ganglion:diagnostics/collect","decision":"allow","reason":"capabilities permitted by policy"}
{"type":"audit_appended","seq":6,"record":{"operator_peer":"12D3-715cfb78…","component_name":"diagnostics","component_version":"0.1.0","capabilities_used":["ganglion:diagnostics/collect@1.0"],"exit":"success","started_at":"2026-08-06T04:42:35.993907211Z","ended_at":"2026-08-06T04:42:36.004347163Z"}}
```

| Flag | Description |
|------|-------------|
| `--follow` | Continuously tail new events (like `tail -f`); Ctrl-C to stop. |
| `--since <dur>` | Only show events newer than this (e.g. `30s`, `5m`, `2h`, `1d`). |
| `-p, --peer <id>` | Explicit peer id (bypasses name/prefix resolution). |
| `-r, --relay <multiaddr>` | Relay multiaddr (overrides registry and config defaults). |

The global `--format json` prints one JSON object per event (JSONL).

> The feed is authenticated: when the robot has a trust store configured, only
> trusted operators may subscribe (the same rule as deploy). Events carry no
> secret material. See [ADR-022](adr/ADR-022-event-subscription-layer.md).

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

### `gang up` (alias: `gang fleet`)

Stand up a **real** local fleet — a loopback circuit relay, a robot agent with a
default-deny policy, and one signed sample capability — then print the exact
commands to drive it from another terminal. This is the bridge between
`gang demo` (self-contained, tears itself down) and a hand-wired
relay/agent/deploy: it is a pure composition of `gang relay`, `gang agent`,
`gang sign`, and `gang peer add` run in-process under one working directory.

The command runs in the foreground and blocks, serving the relay and agent until
you press Ctrl-C, which tears the whole fleet down.

**Flags:**

| Flag | Description |
|------|-------------|
| `--data-dir <path>` | Working directory for the fleet (default: `~/.gang/up`). Holds the operator, relay, and robot identities, the peer registry, the agent policy/trust/state, and the signed sample. This is the global `--data-dir` flag, so pass the same value to `gang --data-dir <path> …` in the second terminal. |
| `--port <port>` | Bind the relay to this loopback TCP port (default: an ephemeral port). |
| `--force` | Reset the data directory if it already exists (removes its keys, registry, and agent state). Without it, `gang up` refuses to clobber a non-empty directory. |
| `--json` | Emit the fleet facts (data dir, relay multiaddr, robot id, sample path, next commands) as JSON for scripting, then keep serving. |

What it does, in order: generates/loads an operator identity plus separate relay
and robot identities; starts the relay and captures its dialable multiaddr;
writes the robot a default-deny `policy.toml` (only the sample's diagnostics
group is permitted, with commented examples for widening it) and a trust store
that trusts the operator; starts the agent pointed at the relay and waits for its
circuit reservation; signs the sample capability (`--capabilities diagnostics`)
with the operator identity; registers the robot as `up-robot` by its dialable id;
and pre-provisions the robot's host key so the printed commands connect without a
TOFU prompt.

```console
$ gang up
=== gang up — standing up a local fleet ===

Data dir: /home/you/.gang/up
Relay circuit reservation established.

  ┌─────────────────────────────────────────────────────────────
  │ Your fleet is up.
  ├─────────────────────────────────────────────────────────────
  │ data dir : /home/you/.gang/up
  │ relay    : /ip4/127.0.0.1/tcp/42139/p2p/12D3KooWNKAAE2Awv9bL7CFyNyZq5dwLzdKZG9S4N78wroekBWNr
  │ robot    : up-robot  (12D3-3bdd18c50e2570ea35114d16e8fd75c8)
  │ sample   : /home/you/.gang/up/diagnostics.wasm  (signed: diagnostics)
  └─────────────────────────────────────────────────────────────

Drive it from another terminal:

  gang --data-dir /home/you/.gang/up deploy up-robot /home/you/.gang/up/diagnostics.wasm
  gang --data-dir /home/you/.gang/up run up-robot diagnostics
  gang --data-dir /home/you/.gang/up caps up-robot
  gang --data-dir /home/you/.gang/up peer list

The agent enforces a default-deny policy (/home/you/.gang/up/robot/policy.toml):
  only the sample's diagnostics group is permitted; any other
  capability group is denied at deploy time.

Ctrl-C tears the whole fleet down.
```

Driven from the second terminal, the printed commands round-trip over the relay
circuit:

```console
$ gang --data-dir ~/.gang/up deploy up-robot ~/.gang/up/diagnostics.wasm
Deployed 'diagnostics' to robot 'up-robot' (via relay)

$ gang --data-dir ~/.gang/up caps up-robot
Capabilities on 'up-robot':
  diagnostics v0.1.0 (by 12D3-3bdd18c50e2570ea35114d16e8fd75c8)
    - ganglion:diagnostics/collect@1.0
```

Deploying a capability that declares a group the policy does not permit is
rejected at deploy time — default-deny is genuinely enforced:

```console
$ gang --data-dir ~/.gang/up sign netprobe.wasm --capabilities network
$ gang --data-dir ~/.gang/up deploy up-robot netprobe.wasm
Error: deploy to 'up-robot' rejected by robot (deploy_failed): capability ganglion:network/probe@1.0 not permitted by policy
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

### `gang doctor`

Print exactly what the network permits. Where `gang diagnose` classifies the
network *archetype*, `gang doctor` answers the field engineer's operational
question directly: which outbound paths Ganglion needs actually work here, is
the relay reachable, and — if not — what is the minimal thing to ask the
customer's network/security team to allow. Ganglion is outbound-only, so every
probe is about egress.

```bash
$ gang doctor
Running outbound reachability probes (this may take a few seconds)...

============================================
  gang doctor — outbound reachability
============================================

  [PASS] Outbound TCP 443
         HTTPS-port egress works — a relay on TCP 443 is reachable from here.
  [FAIL] Outbound UDP (QUIC)
         UDP egress blocked — QUIC won't work; Ganglion will fall back to TCP relay.
  [FAIL] Outbound TCP (non-443)
         Non-443 TCP blocked — enterprise firewall; pin the relay to TCP 443.
  [PASS] DNS resolution
         Name resolution works.
  [PASS] Relay reachability (TCP)
         Relay relay.gang.tafy.dev:443 is reachable.
  [PASS] Operator/robot identity
         Identity key present at ~/.gang/identity.key.

What to tell your network / security team:
  • Ganglion is outbound-only: NO inbound ports need to be opened on the robot's network.
  • Allow outbound TCP to relay.gang.tafy.dev:443 (the Ganglion relay).

Verdict: a viable outbound path exists. You should be able to pair/enroll.
```

Pass `--relay <multiaddr>` to test a specific relay instead of the configured
`default_relay`. Use `--json` (global flag) for machine-readable output. The
command **exits non-zero** when no viable outbound path to a relay exists, so it
works as a gate in scripts and CI and drops cleanly into a support thread:
*"run `gang doctor` and paste the output."*

### `gang profiles`

List the bandwidth profiles available for degraded-link streaming. Profiles are
a transport-shaping concept (how much of an already-permitted stream to
forward), never an access-control one. They are applied with `--profile <name>`
on streaming surfaces such as `gang view`.

```bash
$ gang profiles
Bandwidth profiles (use with --profile <name>):

  full
    No shaping — forward every message at full fidelity.
    decimation 1/1, rate unlimited, per-message cap none

  lidar-low
    Point clouds on a thin link: 1-in-10 messages, ~2 Hz ceiling.
    decimation 1/10, rate 2 Hz, per-message cap none

  vision-low
    Camera/vision topics: 1-in-5 messages, ~1 Hz, 256 KiB/frame cap.
    decimation 1/5, rate 1 Hz, per-message cap 256.0 KB

  logs-only
    Last-resort link: every message but only small (<=16 KiB) payloads.
    decimation 1/1, rate unlimited, per-message cap 16.0 KB
```

Operators can define additional profiles in `~/.gang/config.toml` under
`bandwidth_profiles`; they appear here marked `(custom)`. Built-in names take
precedence. Use `--json` for machine-readable output.

### `gang transport-stats <robot>`

Show REAL per-transport statistics for the live circuit to a robot, read from
the operator transport's connected-peer counters after a probe request. Errors
if the robot is unreachable.

```bash
$ gang transport-stats up-robot
Transport statistics for 'up-robot' (live circuit):
  Transport:       relay
  Via relay:       true
  Connect time:    187ms
  Messages:        1 sent, 1 received
  Bytes:           0 B sent, 173 B received
  Last RTT:        2ms
  DCUtR:           attempted=false, succeeded=false
  Uptime:          0s
  Reconnections:   0
```

| Flag | Description |
|------|-------------|
| `-p, --peer <id>` | Explicit peer id (bypasses name/prefix resolution). |
| `-r, --relay <multiaddr>` | Relay multiaddr (overrides registry and config defaults). |
| `--timeout <secs>` | Overall timeout in seconds (default 30). |

With `--format json` the payload is the raw `TransportStats` plus the resolved
`peer`.

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

```console
$ gang relay
Ganglion Relay Server
====================

Peer ID:      12D3-782c28d3bf62449667fa35b25bf7fdae
Relay mode:   server
Metrics port: 9090 (not yet active)

Listen addresses:
  /ip4/0.0.0.0/tcp/4001
  /ip4/0.0.0.0/udp/4001/quic-v1

Relay multiaddrs (for client config):
  /ip4/0.0.0.0/tcp/4001/p2p/12D3-782c28d3bf62449667fa35b25bf7fdae
  /ip4/0.0.0.0/udp/4001/quic-v1/p2p/12D3-782c28d3bf62449667fa35b25bf7fdae

[log lines] Building Ganglion swarm local_peer_id=12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk relay_server=true
Relay is running. Press Ctrl+C to stop.
```

| Flag | Description |
|------|-------------|
| `--listen-addr <ADDR>` | Multiaddr to listen on. Can be specified multiple times. Default: TCP and QUIC on port 4001. |
| `--port <PORT>` | Port shorthand (sets both TCP and QUIC). Default: `4001`. |
| `--metrics-port <PORT>` | Metrics HTTP port (placeholder). Default: `9090`. |
| `--data-dir <PATH>` | Directory for the relay's persisted identity key. The key path is passed directly to the relay — no environment variable is set or read at relay runtime. Default: `~/.gang/identity.key`. (`GANG_KEY_PATH` is still honored by the default key-path resolution used by other commands.) |

The relay generates or loads an identity from `~/.gang/identity.key` (or
`--data-dir`). It prints both of its identities, labeled: the Ganglion-native
`12D3-<hex>` ID (used in trust stores and policy rules) and the
**libp2p-format** `12D3KooW…` ID (the dialable form). The
`Relay multiaddrs (for client config)` lines carry the dialable form —
copy one of those directly into `gang agent -r` / `gang peer add --relay`.
See `deploy/relay/README.md` for production deployment with Docker.

## Peer management

### `gang peer add <name> <peer-id>`

Register a known peer (robot, relay, or operator). `<peer-id>` accepts either
id form:

- **Dialable libp2p id** (base58 `12D3KooW…`, printed by `gang agent` /
  `gang relay` as `Peer ID (libp2p/dial)`): the gang trust id is derived from
  the Ed25519 key embedded in it, and **both** ids are stored. This is the
  form remote dispatch needs.
- **Legacy gang id** (`12D3-` + 32 hex chars): stored without a dialable id.
  The command notes that remote `deploy`/`run`/`caps` require re-adding the
  peer with the libp2p id.

```bash
$ gang peer add warehouse-bot 12D3KooWK8sozDa46nfm4yhZysi4XRp69QUBuZ8b6M3pza54BNz2 \
    --relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
Registered peer 'warehouse-bot':
  Peer ID (gang identity): 12D3-6ca0419fa75b4ba889669086076df590
  Peer ID (libp2p/dial):   12D3KooWK8sozDa46nfm4yhZysi4XRp69QUBuZ8b6M3pza54BNz2
  Role:    robot-agent
  Relay:   /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

A malformed peer id is rejected with a clean error (exit code 1):

```bash
$ gang peer add badbot not-a-peer-id
Error: Invalid peer ID 'not-a-peer-id'. Expected either the dialable libp2p id (base58 `12D3KooW…`, printed by `gang agent`/`gang relay` at startup) or a gang id (`12D3-` + 32 hex chars).
```

| Flag | Description |
|------|-------------|
| `--relay <multiaddr>`, `-r` | Relay multiaddr for reaching this peer. |
| `--role <role>` | Role: `robot-agent`, `operator`, or `relay`. Default: `robot-agent`. |

### `gang peer remove <name>`

Remove a registered peer.

### `gang peer list`

List all registered peers with their peer IDs, dialable ids, roles, and relay
addresses.

### `gang peer show <name>`

Show details for a specific peer, including both id forms when the dialable
libp2p id is stored.

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
$ gang config set default_relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
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

### `gang list`

List registered robot-agent peers with live reachability from a quick presence
probe over each peer's relay circuit. Reachable robots show their version and
uptime.

```bash
$ gang list
  [up  ] up-robot  12D3-9a5912fe7fc1bfe9393eda322180ccee  v2.1.0, up 12s
```

With `--format json`, prints an array of `{name, peer_id, reachable, detail}`.
Robots with no dialable id / relay, or that are unreachable, are marked `down`
with the reason in `detail`.

### `gang connect <robot>`

Attach a live status view to a robot — presence, heartbeats, connection-state
changes, and a live tail of policy/audit events — as scrolling text. Ctrl-C
detaches. The non-TUI precursor to the `gang tui` dashboard, built on the same
event subscription API.

```bash
$ gang connect up-robot
Connected to 'up-robot'. Live status (Ctrl-C to detach):
presence  v2.1.0  up 12s  archetype=unknown  caps=[diagnostics]
2026-08-06T04:42:46Z  heartbeat  up 0s
2026-08-06T04:42:46Z  conn UP    12D3-463492a3…  transport=direct via_relay=false
2026-08-06T04:42:58Z  policy ALLOW  ganglion:diagnostics/collect  by 12D3-ebbc6e31…  (capabilities permitted by policy)
2026-08-06T04:42:59Z  audit  diagnostics v0.1.0  by 12D3-ebbc6e31…  -> success  caps=[ganglion:diagnostics/collect@1.0]
```

| Flag | Description |
|------|-------------|
| `--prefer-transport <t1,t2>` | Preferred transport order (accepted; reserved for happy-eyeballs selection). |

### `gang tui`

The **live fleet dashboard** — a full-screen [ratatui](https://ratatui.rs)
view built on the same event subscription API as `gang logs`/`connect`
([ADR-022](adr/ADR-022-event-subscription-layer.md)). It subscribes to every
registered robot's event feed and folds it into four panes. The feed defaults to
a genuine server-push substream ([ADR-024](adr/ADR-024-event-push-stream.md)), so
events land the instant the robot emits them, with automatic fallback to the
request-response poll when push is unavailable (`--events-transport
auto|push|poll`). The title bar shows a `feed push` / `feed poll(1.5s)`
indicator for the active transport:

- **Peers** — name, status dot (`●` live / `◐` transitional / `○` offline),
  transport, and RTT.
- **Tunnels** — the operator↔robot circuit: direct vs relay, and ↑/↓ byte
  counters (from the live transport stats `gang transport-stats` reads).
- **Policy decisions (live)** — timestamp, `ALLOW`/`DENY`, capability group,
  operator, and reason, newest last.
- **Audit tail** — timestamp, action, operator, result, and duration.

The title bar shows relay, live/total peer count, dashboard uptime, and a
`♥ live` pulse that flips to `[stale feed]` when no heartbeat arrives within the
15 s liveness window (heartbeats still drive staleness even though the feed
itself is now instant). With **no robots registered** the dashboard shows a friendly
first-run panel pointing at `gang up` / `gang pair` rather than a blank grid.

```console
$ gang --data-dir ~/.gang/up tui
╭ gang tui — fleet dashboard ────────────────────────────────────────────────────╮
│relay /ip4/127.0.0.1/tcp/37163   peers 1/1 live   up 6s                   ♥ live│
╰──────────────────────────────────────────────────────────────────────────────────╯
╭ Peers (1) ──────────────────────╮╭ Tunnels ────────────────────────╮
│   peer          transport  rtt  ││peer       path      ↑ up   ↓ down│
│●  up-robot      relay      3ms  ││up-robot   relay     0 B    3.8 KB │
╰───────────────────────────────────╯╰───────────────────────────────────╯
╭ Policy decisions (live) (3) ────╮╭ Audit tail (1) ─────────────────╮
│05:21:48 ALLOW …/collect  …      ││05:21:48 diagnostics v0.1.0 … ok │
│05:21:49 DENY  …/spawn    …      ││                                 │
╰───────────────────────────────────╯╰───────────────────────────────────╯
↑↓ select · ⏎ inspect · p pause · / filter · a audit · ? help · q quit
```

#### Keybindings

| Key | Action |
|-----|--------|
| `↑` / `↓`, `k` / `j` | Select a peer (wraps). |
| `⏎` Enter | Inspect the selected peer — a drill-down overlay with its capabilities, recent policy decisions, and recent audit. |
| `p` | Pause / resume the live feed. Paused freezes the display and shows a `PAUSED` indicator; buffered events replay on resume — ideal for capturing a clean demo GIF. |
| `/` | Filter by peer name / text (matches peers, decisions, and audit). Enter applies, Esc cancels. |
| `c` | Clear the active filter. |
| `a` | Audit-only fullscreen view. |
| `?` | Toggle the help overlay. |
| `q` / Esc | Quit (Esc first closes an open overlay). Ctrl-C also quits. |

The terminal is always restored on exit — raw mode off, alternate screen left —
even on panic (a panic hook + RAII guard).

#### Flags

| Flag | Description |
|------|-------------|
| `--robot <name>` | Focus a single registered robot instead of the whole fleet. |
| `--frames <n>` | Headless snapshot: fold the live feed for `n` cycles (~1 s each), print the rendered frame as text, then exit. No raw terminal — safe for CI, pipes, and capturing a static frame. |
| `--no-input` | Run the live dashboard but ignore keyboard input (unattended recording); Ctrl-C still quits. |
| `--events-transport <mode>` | Feed transport: `auto` (default; push with poll fallback), `push` (force push), or `poll` (force the request-response poll). See ADR-024. |
| `--data-dir <path>` | (global) Point at a `gang up` fleet directory. |

#### NO_COLOR

`gang tui` honors the [`NO_COLOR`](https://no-color.org) convention: any
non-empty `NO_COLOR` renders a clean **monochrome, ASCII** theme — ASCII box
borders (`+ - |`), ASCII status markers (`* ~ .`), and no color escapes — so
recordings and plain terminals stay legible. The dashboard is resize-aware: on a
narrow terminal it collapses the 2×2 grid into a single stacked column, and
below a minimum size it shows a "terminal too small" hint rather than garbling.

`gang tui` is interactive and does not support `--format json`.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Any error (policy denial, trust/signature failure, I/O, WIP command, …) |
| 2 | Command-line usage error (unknown flag, missing argument — reported by clap) |

Finer-grained exit codes (distinguishing policy denials, trust-store failures,
and signature failures) are planned but not yet implemented.
