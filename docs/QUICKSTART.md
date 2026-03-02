# Quickstart

Get from zero to running Ganglion in under 5 minutes.

## Prerequisites

- Rust 1.85+ (`rustup update stable`)
- Docker (only for test-archetype scenarios)

## 1. Build and install

```bash
git clone https://github.com/tafy-labs/ganglion.git
cd ganglion
cargo install --path crates/gang-cli
```

This puts `gang` on your PATH.

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

## 3. Explore the CLI

```bash
# Show your identity
gang identity show

# Generate a new keypair
gang identity generate

# Run a local robot agent
gang agent --data-dir /tmp/my-robot

# In another terminal, deploy and invoke a capability
gang deploy my-robot path/to/signed.wasm
gang run my-robot diagnostics
```

## 4. Network archetype testing (requires Docker)

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

## 5. What to read next

- [docs/DesignSpec.md](DesignSpec.md) — full architectural design specification
- [docs/VALIDATION.md](VALIDATION.md) — test harness results with measured numbers
- [docs/IMPLEMENTATION.md](IMPLEMENTATION.md) — implementation plan and progress tracking
