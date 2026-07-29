# Quickstart

Get from zero to running Ganglion in under 5 minutes.

## Prerequisites

- Rust 1.88+ (`rustup update stable`)
- System libraries (Debian/Ubuntu): `sudo apt-get install pkg-config libssl-dev`
- Docker (only for test-archetype scenarios)

## 1. Install

Install the CLI from crates.io:

```bash
cargo install gang
```

This puts `gang` on your PATH.

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

The fastest way to see Ganglion work end-to-end:

```bash
gang demo
```

This runs a self-contained demo that:
- Generates two Ed25519 keypairs (operator and robot)
- Creates a signed capability manifest
- Deploys it to a simulated local robot agent
- Invokes the diagnostics capability
- Displays system info, network state, process list, and log sources

No Docker, no ROS 2, no network services required.

## 3. Generate your identity

```bash
# Generate a new Ed25519 keypair
gang identity generate

# Show your peer ID and public key
gang identity show
```

Your keypair is stored at `~/.gang/identity.key`. The peer ID is derived from a Blake3 hash of your public key.

## 4. Diagnose your network

```bash
gang diagnose
```

This runs six network probes and classifies your environment into one of five archetypes (open warehouse, NAT'd office, enterprise DMZ, regulated facility, mobile/CGNAT). It then recommends the appropriate transport configuration.

## 5. Sign and deploy a capability

This walks the full sign → deploy → invoke loop against a *local* robot agent.
No second terminal is needed: `gang deploy`, `gang caps`, and `gang run` spin up
an in-process local agent over a shared data directory
(`/tmp/gang-agent-<robot>`); a separately started `gang agent` process is not
consulted on this path. (Remote dispatch to a real agent over a relay is WIP —
see [CLI_REFERENCE.md](CLI_REFERENCE.md).)

You need an identity (step 3) and a `.wasm` file to sign. `gang sign` signs any
file's bytes, so a placeholder is enough for the walkthrough — building a real
capability is step 6:

```bash
# Create a placeholder component (real ones come from `gang capability scaffold`)
printf '\0asm\1\0\0\0' > my-diagnostics.wasm

# Sign it, declaring its capabilities explicitly
gang sign my-diagnostics.wasm --capabilities diagnostics

# Create the local agent's data directory — this is what makes the name
# 'my-robot' resolve to a local agent at /tmp/gang-agent-my-robot
mkdir -p /tmp/gang-agent-my-robot

# Deploy, list, invoke
gang deploy my-robot my-diagnostics.wasm
gang caps my-robot
gang run my-robot my-diagnostics
```

`gang sign` prints the manifest it produced:

```
Signed component: my-diagnostics.wasm
  Name:     my-diagnostics
  Version:  0.1.0
  Manifest: my-diagnostics.manifest.cbor
  Author:   12D3-6c128fa3aae7bf5eb20105aa8eca5cc0
  Hash:     0d66d411a21e80d93afa1487b002a186...
  Capabilities:
    - ganglion:diagnostics/collect@1.0
```

Deploy and invoke report against the local agent ("[log lines]" summarizes the
agent's tracing output — you'll see permissive-policy and empty-trust-store
warnings, which are expected in this dev flow):

```
$ gang deploy my-robot my-diagnostics.wasm
[log lines]
Deployed 'my-diagnostics' to robot 'my-robot'

$ gang caps my-robot
[log lines]
Capabilities on 'my-robot':
  my-diagnostics v0.1.0 (by 12D3-6c128fa3aae7bf5eb20105aa8eca5cc0)
    - ganglion:diagnostics/collect@1.0

$ gang run my-robot my-diagnostics
[log lines]
System Information:
  Hostname:  vm
  OS:        linux 6.18.5
  Arch:      x86_64
  CPUs:      2
  Memory:    7 GB
  Uptime:    0h 57m
  Ganglion:  v2.0.0
...
```

Clean up with `rm -rf /tmp/gang-agent-my-robot` when done.

## 6. Scaffold a new capability

Generate a capability project skeleton in your language of choice:

```bash
# Rust (default)
gang capability scaffold my-tool

# C++, Python, or Go
gang capability scaffold my-tool --language cpp
gang capability scaffold my-tool --language python
gang capability scaffold my-tool --language go
```

See [CAPABILITY_AUTHOR_GUIDE.md](CAPABILITY_AUTHOR_GUIDE.md) for full authoring instructions.

## 7. Use the content store

```bash
# Push a file to the content-addressed store
gang push /tmp/diagnostics-bundle.tar.gz

# List stored artifacts
gang artifacts

# Fetch an artifact by CID
gang fetch bafy... -o /tmp/retrieved.tar.gz
```

## 8. Browse the registry

```bash
# Search for capabilities
gang registry search diagnostics

# List all registered capabilities
gang registry list

# Get details for a specific capability
gang registry info gang-capability-diagnostics
```

## 9. Network archetype testing (requires Docker)

Test Ganglion across simulated hostile network conditions:

```bash
# Simple flat network — direct connectivity
gang test-archetype open-warehouse

# Behind consumer NAT — relay + hole-punching
gang test-archetype nat-office

# Enterprise firewall — TCP 443 only, VLAN isolation
gang test-archetype enterprise-dmz

# Cellular/CGNAT — symmetric NAT, jitter, packet loss
gang test-archetype mobile-cgnat
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

## What to read next

- [ARCHITECTURE.md](ARCHITECTURE.md) — full architectural reference (three layers, crate map, data flows)
- [SECURITY.md](SECURITY.md) — threat model, trust boundaries, security mechanisms
- [CLI_REFERENCE.md](CLI_REFERENCE.md) — complete CLI documentation with all flags and examples
- [NETWORK_ARCHETYPES.md](NETWORK_ARCHETYPES.md) — deep dive on the five network archetypes
- [CAPABILITY_AUTHOR_GUIDE.md](CAPABILITY_AUTHOR_GUIDE.md) — writing capabilities in Rust, C++, Python, Go
- [DesignSpec.md](DesignSpec.md) — original design specification
- [VALIDATION.md](VALIDATION.md) — test harness results and measurements
