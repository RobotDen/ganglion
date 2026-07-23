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
git clone https://github.com/TafyLabs/ganglion.git
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

```bash
# Sign a WASM component (declare its capabilities explicitly)
gang sign my-diagnostics.wasm --name my-diagnostics \
    --component-version 0.1.0 --capabilities diagnostics,logs

# Start a local robot agent
gang agent --data-dir /tmp/my-robot

# In another terminal, deploy and invoke
gang deploy my-robot my-diagnostics.wasm
gang run my-robot my-diagnostics
```

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
