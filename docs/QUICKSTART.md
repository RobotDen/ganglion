# Quickstart

From zero to a signed capability running under policy — plus a relay and a
relay-connected robot agent. Every transcript below is real captured output,
lightly trimmed; `[log lines]` marks elided tracing output.

For a one-paragraph summary of which parts of the fleet path are done and which
are in progress, see [Fleet status: what works
today](../README.md#fleet-status-what-works-today) in the README. Short
version: everything below works as shown; the one unfinished hop is
operator-side *remote* dispatch (step 8 shows the exact error you get and the
local path that works today).

## Prerequisites

- Rust 1.88+ (`rustup update stable`)
- System libraries (Debian/Ubuntu): `sudo apt-get install pkg-config libssl-dev`
- Docker (only for test-archetype scenarios)

## 1. Install

Install the CLI from crates.io:

```bash
cargo install gang
```

This puts `gang` on your PATH. Install it on both the workstation and the
robot — the same binary provides the operator CLI, the robot agent, and the
relay server.

### From source (contributors)

```bash
git clone https://github.com/RobotDen/ganglion.git
cd ganglion

# Set up git hooks (recommended for development)
./scripts/setup-hooks.sh

# Install the CLI from the checkout
cargo install --path crates/gang-cli
```

## 2. Run the demo

The fastest way to see the whole pipeline — keygen, signing, deploy, policy
check, sandboxed invoke, audit log — with no Docker, no ROS 2, no network:

```console
$ gang demo
=== Ganglion v2.0.0 Demo ===

Operator identity: 12D3-c2ace1a32fd67c0c8c66976336bceead
Robot agent:       12D3-44f2a6a68b5e33b7086ec7e183b26d05

--- Signing diagnostics capability ---
  Component signed by 12D3-c2ace1a32fd67c0c8c66976336bceead

--- Deploying to robot ---
[log lines]
  Deployed: diagnostics

--- Installed capabilities ---
  diagnostics v0.1.0 (12D3-c2ace1a32fd67c0c8c66976336bceead)

--- Invoking diagnostics ---
System Information:
  Hostname:  vm
  OS:        linux 6.18.5
  Arch:      x86_64
  CPUs:      2
  Memory:    7 GB
  Uptime:    0h 29m
  Ganglion:  v2.0.0

Processes: 65 running
  PID 29222: 100.0% CPU — gang demo
  ...

Log Sources:
  journald (journald)

--- Audit log ---
  12D3-c2ace1a32fd67c0c8c66976336bceead invoked 'diagnostics' v0.1.0 at 18:47:54 -> Success

=== Demo complete ===
Data stored at: /tmp/gang-demo
Clean up when done: rm -rf /tmp/gang-demo
```

The diagnostics output (hostname, OS, process list) is collected live from the
machine you run it on.

## 3. How robots are reached

Before deploying anything, know how targeting works — none of it involves DNS.

**Identity, not hostnames.** Every party (operator, robot, relay) has an
Ed25519 keypair. A robot *is* its peer ID: `12D3-` followed by 32 hex
characters, derived from a Blake3 hash of the public key (e.g.
`12D3-f721be4d302e7da31bebf3b89e2b9f53`). The same key also yields a
libp2p-format peer ID (base58, `12D3KooW...`) which appears in transport logs
and multiaddrs. Names like `my-robot` are **local aliases** you assign; they
live in `~/.gang/peers.json` on your workstation and resolve to peer IDs, never
through DNS.

**Outbound only.** Robots sit behind NATs and firewalls you don't control, so
they never accept inbound connections. Instead, a robot agent dials out to a
**relay** (a libp2p circuit relay v2 server) on a host both sides can reach,
and operators reach the robot through that relay circuit (upgrading to a direct
connection via DCUtR hole-punching when the network allows).

**Addresses are multiaddrs.** A relay is identified by a multiaddr — transport
path plus peer identity in one string:

```
/ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

That reads: dial TCP to 203.0.113.10:4001 and expect the peer with that libp2p
ID (the connection fails if a different key answers — the address embeds the
authentication).

**Name resolution order.** When you run `gang deploy <target> ...` (or `run`,
`caps`), the target resolves in this order:

1. **Registered peer name** — an alias created with
   `gang peer add <name> <peer-id> --relay <multiaddr> --role robot-agent`
   (stored in `~/.gang/peers.json`)
2. **Peer-ID prefix** — Docker-style abbreviation, e.g. `12D3-f721`
   (must be unambiguous among registered peers)
3. **Full peer ID** — `12D3-` + 32 hex chars
4. **Local agent fallback** — if `/tmp/gang-agent-<target>` exists, the command
   runs against an in-process local agent over that directory (step 8)

**Host keys (TOFU).** Peer identity verification is SSH-style
trust-on-first-use: `host_key_policy` in `~/.gang/config.toml` is `strict`
(prompt on first connect, hard-fail on key change), `tofu` (auto-accept new
keys, hard-fail on change), or `none`. `gang peer trust-reset <name>` clears a
stored host key. The policy is stored and the verification machinery exists
today; it is enforced on the remote-dispatch path as that lands (see the
[Fleet status](../README.md#fleet-status-what-works-today) box).

**Know your network.** `gang diagnose` probes the local environment and
classifies it into one of five archetypes. Real output from the (deliberately
network-less) machine these transcripts were captured on:

```console
$ gang diagnose
Running network probes...

  Detected:    regulated-facility (80% confidence)

Probes:
  ✗ internet_connectivity: No outbound internet detected — possible air-gap or strict firewall
  ✗ outbound_ports: Non-443 outbound port blocked — possible enterprise firewall
  [log lines]

Recommendations:
  → No network connectivity detected — use offline signed bundles
  → Pre-sign capabilities on an external machine
  → Transfer via USB or approved sneakernet process
```

On an office network you'll typically see `nat-office` and a recommendation to
configure a relay.

## 4. Generate your identity

```console
$ gang identity generate
Generated new identity:
  Peer ID:  12D3-c2ace1a32fd67c0c8c66976336bceead
  Key file: ~/.gang/identity.key
```

The keypair lands at `~/.gang/identity.key` (mode 0600). If you ran `gang demo`
first, an identity already exists (the demo creates one on first use) and
`generate` refuses to overwrite it without `--force` — `gang identity show`
prints the peer ID and public key either way. This operator identity signs
every capability you produce and appears in every audit-log entry on the robot.

## 5. Start a relay

On a server reachable by both your workstation and the robots:

```console
$ gang relay --port 4001 --data-dir /var/lib/gang-relay
Ganglion Relay Server
====================

Peer ID:      12D3-782c28d3bf62449667fa35b25bf7fdae
Relay mode:   server

Listen addresses:
  /ip4/0.0.0.0/tcp/4001
  /ip4/0.0.0.0/udp/4001/quic-v1

[log lines] Building Ganglion swarm local_peer_id=12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk relay_server=true
Relay is running. Press Ctrl+C to stop.
```

Build the dialable relay multiaddr from your server's address plus the
**libp2p-format** peer ID from the `local_peer_id=` log line:

```
/ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

> **Note:** the `Relay multiaddrs (for client config)` lines the relay also
> prints embed the short `12D3-<hex>` (Ganglion-native) peer ID; multiaddr
> parsing only accepts the base58 `12D3KooW...` form, so use the
> `local_peer_id` value in the `/p2p/` component. (Known gap — the printed
> config lines will switch to the dialable form.)

The relay's identity key persists in `--data-dir`, so the multiaddr stays
stable across restarts. See `deploy/relay/README.md` for production deployment
with Docker.

## 6. Connect a robot agent

On the robot, run the agent pointing at the relay. It dials out — no inbound
port, no firewall change on the robot's network:

```console
$ gang agent --data-dir /var/lib/gang-agent \
    -r /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
[log lines]
Robot agent started:
  Peer ID:  12D3-f721be4d302e7da31bebf3b89e2b9f53
  Data dir: /var/lib/gang-agent
  Policy:   permissive (dev mode)
  Relay:    /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
  Mode:     remote (listening on /ganglion/control/1.0)

Register on operator machine:
  gang peer add my-robot 12D3-f721be4d302e7da31bebf3b89e2b9f53 --relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk

Starting transport...
[log lines]
Connected to relay. Waiting for operator connections...
```

If the relay is unreachable the agent doesn't exit — it logs a warning and
retries every 5 seconds. Without a policy file the agent warns loudly that it
is running a PERMISSIVE dev policy with trust checks disabled; production
agents get a default-deny policy file and a populated trust store (see
[SECURITY.md](SECURITY.md)).

## 7. Register the robot on your workstation

Paste the line the agent printed (naming it whatever you like):

```console
$ gang peer add robot-a 12D3-f721be4d302e7da31bebf3b89e2b9f53 \
    --relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk \
    --role robot-agent
Registered peer 'robot-a':
  Peer ID: 12D3-f721be4d302e7da31bebf3b89e2b9f53
  Role:    robot-agent
  Relay:   /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

From now on `robot-a` resolves to that peer ID + relay (`gang peer list` shows
the mapping). This is the entire "establishing a connection to a robot" story:
robot dials relay, you record the robot's identity and relay locally.

## 8. Sign, deploy, run

Sign a component with your identity, declaring its capabilities explicitly.
`gang sign` signs any file's bytes, so a placeholder is enough for the
walkthrough — building a real component is step 9:

```console
$ printf '\0asm\1\0\0\0' > my-diagnostics.wasm
$ gang sign my-diagnostics.wasm --capabilities diagnostics
Signed component: my-diagnostics.wasm
  Name:     my-diagnostics
  Version:  0.1.0
  Manifest: my-diagnostics.manifest.cbor
  Author:   12D3-c2ace1a32fd67c0c8c66976336bceead
  Hash:     0d66d411a21e80d93afa1487b002a186...
  Capabilities:
    - ganglion:diagnostics/collect@1.0
```

**The honest caveat.** Deploying to the *remote* `robot-a` is the one hop that
is not finished yet — the operator side does not send the request over the
relay circuit. It fails fast and tells you so:

```console
$ gang deploy robot-a my-diagnostics.wasm
Error: Remote deploy to 'robot-a' (12D3-f721be4d302e7da31bebf3b89e2b9f53) is not yet implemented.
The transport infrastructure is ready but the agent serve loop (ADR-020 Phase 32)
must be completed first. Use local mode for now
```

The same applies to `gang run` and `gang caps` against remote targets, and
`gang logs`/`list`/`connect`/`transport-stats` depend on the same hop. Status
and tracking: [Fleet status](../README.md#fleet-status-what-works-today) and
[ADR-020](adr/ADR-020-remote-dispatch-and-e2e-test.md).

**The local path works end-to-end** and exercises the identical
signing/policy/audit pipeline. Creating `/tmp/gang-agent-<name>` is what makes
a name resolve to a local in-process agent (resolution rule 4 in step 3):

```console
$ mkdir -p /tmp/gang-agent-my-robot

$ gang deploy my-robot my-diagnostics.wasm
[log lines]
Deployed 'my-diagnostics' to robot 'my-robot'

$ gang caps my-robot
[log lines]
Capabilities on 'my-robot':
  my-diagnostics v0.1.0 (by 12D3-c2ace1a32fd67c0c8c66976336bceead)
    - ganglion:diagnostics/collect@1.0

$ gang run my-robot my-diagnostics
[log lines]
System Information:
  Hostname:  vm
  OS:        linux 6.18.5
  Arch:      x86_64
  CPUs:      2
  Memory:    7 GB
  Uptime:    0h 29m
  Ganglion:  v2.0.0

Processes: 66 running
  ...
```

The `[log lines]` include the same permissive-policy and empty-trust-store
warnings as step 6 — expected in this dev flow. Clean up with
`rm -rf /tmp/gang-agent-my-robot` when done.

## 9. Author a real capability

The repo's eight `gang-capability-*` crates (including the diagnostics
capability the demo embeds) are real WASM components; your own follow the same
scaffold → build → sign → deploy pipeline:

```console
$ gang capability scaffold my-tool
Scaffolded rust capability at ./my-tool

Next steps:
  1. Implement your capability logic (WIT is in my-tool/wit/ganglion.wit)
  2. Build: see docs/CAPABILITY_AUTHOR_GUIDE.md
  3. Sign: gang sign my-tool.component.wasm --name my-tool --version 0.1.0
```

The generated Makefile drives the rest (requires
`rustup target add wasm32-wasip2` and
[wasm-tools](https://github.com/bytecodealliance/wasm-tools)):

```bash
cd my-tool
make component   # cargo build --target wasm32-wasip2 --release + wasm-tools component new
make sign        # gang sign my-tool.component.wasm ...
gang deploy my-robot my-tool.component.wasm
```

C++, Python, and Go scaffolds exist too (`--language cpp|python|go`). See
[CAPABILITY_AUTHOR_GUIDE.md](CAPABILITY_AUTHOR_GUIDE.md) for full authoring
instructions and [examples/](../examples/) for reference implementations.

## 10. Content store and registry

```bash
# Content-addressed artifact store
gang push /tmp/diagnostics-bundle.tar.gz   # -> prints a CID
gang artifacts                             # list stored artifacts
gang fetch bafy... -o /tmp/retrieved.tar.gz

# Local capability registry
gang registry search diagnostics
gang registry list
gang registry info gang-capability-diagnostics
```

## 11. Network archetype testing (requires Docker)

Test Ganglion across simulated hostile network conditions:

```bash
gang test-archetype open-warehouse   # flat network — direct connectivity
gang test-archetype nat-office       # consumer NAT — relay + hole-punching
gang test-archetype enterprise-dmz   # TCP 443 only, VLAN isolation
gang test-archetype mobile-cgnat     # symmetric NAT, jitter, packet loss
```

Each scenario builds container images, starts the network topology, and shows service status. You can then exec into containers to inspect:

```bash
docker compose -p ganglion-nat-office -f test-harness/nat-office/docker-compose.yml exec robot bash
docker compose -p ganglion-nat-office -f test-harness/nat-office/docker-compose.yml logs -f
```

Tear down when done:

```bash
docker compose -p ganglion-nat-office -f test-harness/nat-office/docker-compose.yml down -v
```

Measured results across the scenarios are in [VALIDATION.md](VALIDATION.md).

## What to read next

- [ARCHITECTURE.md](ARCHITECTURE.md) — full architectural reference (three layers, crate map, data flows)
- [SECURITY.md](SECURITY.md) — threat model, trust boundaries, security mechanisms
- [CLI_REFERENCE.md](CLI_REFERENCE.md) — complete CLI documentation with all flags and examples
- [NETWORK_ARCHETYPES.md](NETWORK_ARCHETYPES.md) — deep dive on the five network archetypes
- [CAPABILITY_AUTHOR_GUIDE.md](CAPABILITY_AUTHOR_GUIDE.md) — writing capabilities in Rust, C++, Python, Go
- [DesignSpec.md](DesignSpec.md) — original design specification
- [VALIDATION.md](VALIDATION.md) — test harness results and measurements
