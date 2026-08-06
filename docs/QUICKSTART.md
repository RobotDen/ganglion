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

## 1b. Configure in one command: `gang init`

Before wiring anything by hand, go from *installed* to *configured* in one step.
`gang init` detects your network archetype (the same probes as `gang diagnose`),
generates your operator identity, writes a **default-deny** `policy.toml` with
commented example rules and an operator `config.toml`, and prints exactly what to
run next. It is interactive on a terminal and fully non-interactive in CI or a
pipe (or with `-y`); re-running never clobbers an existing identity, policy, or
config without `--force`.

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

  # Enrol a robot in one line (operator waits; robot runs the printed line):
  gang pair --relay <relay-multiaddr> --name <name>
  #   ... or register manually: gang peer add <name> <robot-libp2p-id> --relay <relay-multiaddr>

Run `gang status` to review your configuration.
```

The archetype above is `regulated-facility` only because the machine capturing
this transcript had no network; on a networked host the panel prints a
`gang relay` / `gang agent` / `gang peer add` / `gang deploy` sequence instead.
The manual steps in sections 3–8 are exactly what that panel points at, wired by
hand. Full flag reference: [`gang init` in the CLI
reference](CLI_REFERENCE.md#gang-init).

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

## 2b. One-command local fleet: `gang up`

`gang demo` runs everything in a single process and tears itself down when it
finishes — there is nothing left running to send commands to. `gang up` is the
bridge to a real deployment: it stands up a **live** local fleet you drive with
the same `gang` commands you'd use against a robot in the field, but entirely on
loopback. It is a straight composition of the manual steps in sections 5–8
below (relay, agent, sign, peer add), collapsed into one command.

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

`gang up` blocks in the foreground while the relay and agent serve; open a
second terminal and paste the printed commands. Everything under `--data-dir`
(default `~/.gang/up`) is self-contained — a separate operator identity, relay
identity, and robot identity, plus the peer registry and the signed sample. The
global `--data-dir` flag points the whole CLI at that directory, so
`gang --data-dir <dir> …` in the second terminal reads exactly the files `up`
wrote. Press Ctrl-C in the first terminal to shut the fleet down; pass `--force`
to reset an existing fleet directory, or `--json` to emit the fleet facts for
scripting.

**Default-deny is real.** The agent loads a restrictive policy from disk (not
the permissive dev fallback). The sample declares only
`ganglion:diagnostics/collect`, which the policy permits, so it deploys and
runs. Sign a capability that declares any other group and the robot rejects it
at deploy time:

```console
$ gang --data-dir ~/.gang/up sign netprobe.wasm --capabilities network
$ gang --data-dir ~/.gang/up deploy up-robot netprobe.wasm
Error: deploy to 'up-robot' rejected by robot (deploy_failed): capability ganglion:network/probe@1.0 not permitted by policy
```

Edit `<data-dir>/robot/policy.toml` (it ships with commented examples) to widen
what the fleet permits.

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
2. **Peer-ID prefix** — Docker-style abbreviation, e.g. `12D3-6ca0`
   (must be unambiguous among registered peers)
3. **Full peer ID** — either form: the dialable base58 libp2p id
   (`12D3KooW…`) or the gang id (`12D3-` + 32 hex chars; not dialable, so
   remote dispatch needs the libp2p form)
4. **Local agent fallback** — if `/tmp/gang-agent-<target>` exists, the command
   runs against an in-process local agent over that directory (step 8)

**Two id forms.** A robot has one Ed25519 key and two spellings of its
identity: the base58 **libp2p id** (`12D3KooW…`) literally embeds the public
key and is the only form a `/p2p/` multiaddr component accepts, and the short
**gang id** (`12D3-<32 hex>`, a Blake3 digest of the same key) used in trust
stores, policies, and audit logs. Register robots with the libp2p id — the
gang id is derived from it automatically and both are stored.

**Host keys (TOFU).** Peer identity verification is SSH-style
trust-on-first-use, enforced on every remote `deploy`/`run`/`caps`:
`host_key_policy` in `~/.gang/config.toml` is `strict` (prompt on first
connect, hard-fail on key change; requires an interactive terminal), `tofu`
(auto-accept new keys, hard-fail on change — use this for scripts), or `none`.
`gang peer trust-reset <name>` clears a stored host key after an expected
re-image.

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

Peer ID (gang identity): 12D3-782c28d3bf62449667fa35b25bf7fdae
Peer ID (libp2p/dial):   12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
Relay mode:   server

Listen addresses:
  /ip4/0.0.0.0/tcp/4001
  /ip4/0.0.0.0/udp/4001/quic-v1

Relay multiaddrs (for client config):
  /ip4/0.0.0.0/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
  /ip4/0.0.0.0/udp/4001/quic-v1/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk

Relay is running. Press Ctrl+C to stop.
```

The `/p2p/` component carries the **libp2p-format** (base58) peer id — the
only dialable form. Substitute your server's address for `0.0.0.0`:

```
/ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

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
  Peer ID:  12D3-6ca0419fa75b4ba889669086076df590
  Data dir: /var/lib/gang-agent
  Policy:   permissive (dev mode)
  Relay:    /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
  Mode:     remote (listening on /ganglion/control/1.0)

[log lines] Requesting relay circuit reservation on /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i…/p2p-circuit
Peer ID (libp2p/dial): 12D3KooWK8sozDa46nfm4yhZysi4XRp69QUBuZ8b6M3pza54BNz2

Register on operator machine:
  gang peer add my-robot 12D3KooWK8sozDa46nfm4yhZysi4XRp69QUBuZ8b6M3pza54BNz2 --relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk

Starting transport...
[log lines]
Relay circuit reservation established.
Connected to relay. Waiting for operator connections...
```

The circuit **reservation** the agent requests is what makes the robot
reachable *through* the relay — merely being connected is not enough. If the
relay is unreachable the agent doesn't exit — it logs a warning, retries every
5 seconds, and re-establishes the reservation. Without a policy file the agent
warns loudly that it is running a PERMISSIVE dev policy with trust checks
disabled; production agents get a default-deny policy file and a populated
trust store (see [SECURITY.md](SECURITY.md)).

## 7. Enrol the robot in one line: `gang pair`

The Tailscale move. On your **workstation**, run `gang pair` and it prints one
line to run on the robot. When the robot runs it, the robot dials out, enrolls,
and appears in your peer list — no copying identifiers in either direction.

```console
$ gang pair --relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk --name robot-a
=== gang pair — enroll a robot in one line ===

Relay:    /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
Operator: 12D3-74aded42b4ed2b3e88d1a4cedd8ec501
Expires:  2026-08-06T03:50:13.922+00:00

Run this ONE line on the robot:

    gang join gang1_pWd2ZXJzaW9uAWpyZWxheV9hZGRyeFEvaXA0…

Waiting up to 300s for the robot to dial out and enroll…
```

On the **robot**, paste that one line:

```console
$ gang join gang1_pWd2ZXJzaW9uAWpyZWxheV9hZGRyeFEvaXA0…
Joining fleet via /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i…

  ✔ joined: registered with operator 12D3-74aded42b4ed2b3e88d1a4cedd8ec501 as 'robot-a'
    this robot: 12D3-6ca0419fa75b4ba889669086076df590

Serving on the relay circuit. Press Ctrl-C to stop.
```

Back on the workstation, `gang pair` confirms and exits; the robot is now a
normal fleet member. The operator records the robot under the identity **libp2p
authenticated on the wire** — never a self-report — so a robot can only enrol as
an identity whose key it holds. The token is single-use and expiring. See
[ADR-021](adr/ADR-021-pairing-token-enrollment.md) for the trust model.

### Manual fallback: `gang peer add`

If you can't run `gang join` on the robot (air-gapped, scripted provisioning),
paste the line the agent printed at startup instead:

```console
$ gang peer add robot-a 12D3KooWK8sozDa46nfm4yhZysi4XRp69QUBuZ8b6M3pza54BNz2 \
    --relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk \
    --role robot-agent
Registered peer 'robot-a':
  Peer ID (gang identity): 12D3-6ca0419fa75b4ba889669086076df590
  Peer ID (libp2p/dial):   12D3KooWK8sozDa46nfm4yhZysi4XRp69QUBuZ8b6M3pza54BNz2
  Role:    robot-agent
  Relay:   /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk
```

Note the gang identity was **derived** from the dialable id you pasted — the
base58 libp2p id embeds the robot's Ed25519 public key. From now on `robot-a`
resolves to that identity + relay (`gang peer list` shows the mapping).

## 8. Sign, deploy, run

Sign a component with your identity, declaring its capabilities explicitly.
`gang sign` signs any file's bytes, so a placeholder is enough for the
walkthrough — building a real component is step 9:

```console
$ printf 'demo capability bytes (not wasm)' > my-tool.wasm
$ gang sign my-tool.wasm --name my-tool --capabilities diagnostics
Signed component: my-tool.wasm
  Name:     my-tool
  Version:  0.1.0
  Manifest: my-tool.manifest.cbor
  Author:   12D3-68c62c79b89c56c575df0845f26b6fae
  Hash:     b4f4a1814d6cedd002496f90a01b469e...
  Capabilities:
    - ganglion:diagnostics/collect@1.0
```

**Deploy over the relay circuit.** The operator dials the relay, requests a
circuit to the robot, verifies the robot's host key (TOFU), and sends the
signed bundle on `/ganglion/control/1.0`. Real (trimmed) output — this run
used `host_key_policy = tofu`, so the first connect auto-records the key:

```console
$ gang deploy robot-a my-tool.wasm
Auto-accepted host key for 12D3-6ca0419fa75b4ba889669086076df590 (fingerprint: BLAKE3:6ca0419fa75b4ba889669086076df590).
Warning: Permanently added '12D3-6ca0419fa75b4ba889669086076df590' (BLAKE3:6ca0419fa75b4ba889669086076df590) to the list of known robots.
Deployed 'my-tool' to robot 'robot-a' (via relay)

$ gang caps robot-a
Capabilities on 'robot-a':
  my-tool v0.1.0 (by 12D3-68c62c79b89c56c575df0845f26b6fae)
    - ganglion:diagnostics/collect@1.0

$ gang run robot-a my-tool
System Information:
  Hostname:  vm
  OS:        linux 6.18.5
  Arch:      x86_64
  CPUs:      2
  Memory:    7 GB
  ...
```

Every request carries a fresh nonce + timestamp (the robot rejects stale or
replayed requests), the whole dispatch is timeout-bounded (60 s deploy, 30 s
run/caps, `--timeout <secs>` to override), and any remote failure — robot
offline, policy denial, signature failure — exits non-zero with the robot's
actual error. A robot that is down looks like this:

```console
$ gang caps robot-a --timeout 8
Error: timed out after 8s: robot 'robot-a' not reachable via relay /ip4/203.0.113.10/tcp/4001/p2p/12D3KooWMc1i6BT7WVRKoC2hpuqThpWdxTfFZ833MCMBdm2L3xuk (is the agent running, and did it connect to that relay?)
```

`gang logs`/`list`/`connect`/`transport-stats` still wait on the
presence/streaming layer — status and tracking:
[Fleet status](../README.md#fleet-status-what-works-today) and
[ADR-020](adr/ADR-020-remote-dispatch-and-e2e-test.md).

**The local path also works end-to-end** and exercises the identical
signing/policy/audit pipeline with no network at all. Creating
`/tmp/gang-agent-<name>` is what makes a name resolve to a local in-process
agent (resolution rule 4 in step 3):

```console
$ mkdir -p /tmp/gang-agent-my-robot

$ gang deploy my-robot my-tool.wasm
[log lines]
Deployed 'my-tool' to robot 'my-robot'

$ gang caps my-robot
[log lines]
Capabilities on 'my-robot':
  my-tool v0.1.0 (by 12D3-68c62c79b89c56c575df0845f26b6fae)
    - ganglion:diagnostics/collect@1.0

$ gang run my-robot my-tool
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
