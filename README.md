<img src="assets/logo-lockup.svg" alt="Ganglion" height="72">

# Ganglion

Secure connectivity and auditable tool execution for ROS 2 robot fleets on hostile networks — outbound-only reachability, signed WASM tooling, default-deny policy.

![status: usable](https://img.shields.io/badge/status-usable-green) [![crates.io](https://img.shields.io/crates/v/gang.svg)](https://crates.io/crates/gang) [![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

![gang demo](docs/assets/ganglion-demo.gif)

Reach robots deployed inside customer networks you don't own and can't configure — and run only signed, sandboxed, policy-bounded, audited tooling on them. Ganglion is a connectivity **and** governed-execution substrate. It is not a fleet-management platform, not a robot-autonomy framework, and not a SaaS product.

## Where Ganglion fits

Connectivity tools answer one question: *"Can I reach the robot?"* Ganglion answers a second one they don't: *"Can I reach it **and** run only signed, sandboxed, policy-bounded, audited tooling on it?"* VPNs and overlays are network planes; Ganglion is a **governed execution substrate** that brings its own hostile-network reachability and composes with whatever runs on the robot.

**Already on Tailscale + SSH? Keep them.** Ganglion isn't another VPN to rip-and-replace — it's the governance layer those tools don't provide. A VPN gives anyone who connects ambient shell authority; Ganglion turns "reach the machine" into "run *this specific signed tool*, once, under a default-deny policy, with a tamper-evident record of who ran what." The two compose: your VPN for the humans who need a shell, Ganglion for the automated tooling that shouldn't get one.

| Tool | What it solves | What it doesn't | Reach for Ganglion when |
|------|----------------|-----------------|-------------------------|
| SSH + VPN (Tailscale/WireGuard) | Machine-level network reachability | Anyone connected has ambient shell authority | "Who ran what on the robot?" must have a provable answer |
| Husarnet | ROS-friendly P2P VPN across sites | Governance of what executes once connected | You need per-tool grants, not network membership |
| OpenZiti | Zero-trust overlay for service access | No opinion on code that runs after connect | The unit to authorize is a tool run, not a network flow |
| Zenoh | Scalable pub/sub/query data plane, DDS-over-WAN | Signed, sandboxed, audited execution | You need governed actions, not (just) data flow |
| DDS (Fast DDS/Cyclone) | In-robot and LAN real-time pub/sub | NAT/CGNAT traversal; any execution model | Robots sit behind networks DDS discovery can't cross |
| PyZeROS | Python ROS 2 clients over Zenoh, no ROS install | Security, policy, audit (not its goal) | Someone else's laptop must run bounded tooling remotely |
| Foxglove | Robotics observability and visualization | Executing tooling on the robot | You need to act, not only observe (they compose) |
| Viam / Transitive | Platform to build robot apps on | Default-deny signed tooling for existing ROS 2 stacks | You keep ROS 2 and add governance, not re-platform |

Full depth, "use both" patterns, and an honest "Ganglion is *not* for you if…" list: **[docs/COMPARISON.md](docs/COMPARISON.md)**.

## Quick start

```bash
# Install — prebuilt binary, no Rust toolchain needed (Linux & macOS, x86_64 & arm64).
# Verifies its SHA-256 before installing. (Or: cargo install gang)
curl -fsSL https://raw.githubusercontent.com/RobotDen/ganglion/main/scripts/install.sh | sh
```

Then, in order:

```bash
gang demo    # 60-second proof: keygen → sign → deploy → policy → sandboxed invoke → audit,
             # all in one process. No Docker, no ROS 2, no network.

gang up      # A REAL local fleet: loopback relay + robot agent + one signed sample,
             # under a default-deny policy. Prints the exact commands to drive it.

gang tui     # The live dashboard — peers, tunnels, policy allow/deny, and a tailing
             # audit log, folded from a genuine server-push event feed.
```

`gang up` prints a `--data-dir`; point `gang tui --data-dir <that>` at it in a second terminal to watch the fleet you just started.

That is the whole local loop. To go to a **real multi-host deployment** — stand up a relay, connect a robot that dials out, and enrol it in one line with `gang pair` / `gang join` — follow the full walkthrough with real transcripts in **[docs/QUICKSTART.md](docs/QUICKSTART.md)**. Every command and flag is in **[docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md)**.

## What works today

Everything local, the connectivity layer, **and** the remote dispatch hop are built and verified: `gang demo`, identity, component signing, the policy/audit pipeline, the relay server, robot agents that dial out and hold a circuit reservation, the peer registry, and **remote `deploy`/`run`/`caps` over the relay circuit** with SSH-style TOFU host-key verification, replay-protected control, and honest non-zero exits.

The **presence/streaming layer** is built too: an authenticated, bounded robot→operator event feed (presence, policy decisions, audit appends, connection changes, heartbeats) powers `gang logs`, `gang connect`, `gang transport-stats`, `gang list`, and the `gang tui` dashboard. The feed defaults to a **genuine server-push substream** over `/ganglion/events/1.0` (libp2p-stream) — events reach operators the instant the robot emits them — with the request-response poll **retained as an automatic fallback** (`--events-transport auto|push|poll`), because libp2p-stream is pre-release. See [ADR-024](docs/adr/ADR-024-event-push-stream.md).

## Architecture

Three layers:

1. **Connectivity** — libp2p peer identity, secure channels (Noise), TCP + QUIC, circuit relay v2, NAT traversal ([DCUtR hole-punching](https://bford.info/pub/net/p2pnat/#3.2%20Establishing%20Peer-to-Peer%20Sessions)). Robots dial out; operators reach them via a shared relay. Inbound connectivity is never assumed.
2. **Tool execution** — a WASM component runtime (Wasmtime). Signed, sandboxed, versioned tools with explicit capability declarations. No ambient authority; memory limits, CPU budgets, and wall-clock deadlines enforced per component.
3. **Native brokers** — privileged host processes that mediate WASM capability access to ROS 2 topics/services/parameters, filesystem, logs, diagnostics, subprocess execution, network probing, and metrics. **Pattern-based access gating with a default-deny policy.**

Full reference: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Security model

Ed25519 identity (PeerId = Blake3 of the public key) · Noise-encrypted channels · signed component manifests verified against a trust store · a **default-deny** policy engine that fails closed on malformed input · tamper-evident Blake3 hash-chain audit log · replay protection on control requests · a TOCTOU-closed symlink jail, command allowlisting, and SSRF guards on the brokers · fuel/memory/deadline limits on every WASM execution.

Full threat model: [docs/SECURITY.md](docs/SECURITY.md).

## Learn more

- **[docs/QUICKSTART.md](docs/QUICKSTART.md)** — the full getting-started walkthrough with real transcripts.
- **[docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md)** — every command, flag, and example.
- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — the three layers, the eight WIT capability groups, and the crate map.
- **[docs/NETWORK_ARCHETYPES.md](docs/NETWORK_ARCHETYPES.md)** — the five network environments Ganglion is designed around (open warehouse, NAT'd office, enterprise DMZ, regulated facility, mobile/CGNAT).
- **[docs/CAPABILITY_AUTHOR_GUIDE.md](docs/CAPABILITY_AUTHOR_GUIDE.md)** — writing capabilities in Rust, C++, Python, or Go.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — building from source and the test harness.

## Support & production

Ganglion is Apache-2.0 — run it, fork it, no strings. If it saves you a truck roll, **[sponsoring the den](https://github.com/sponsors/karma0)** is a welcome thanks and keeps the open core moving.

When one engineer's setup becomes a fleet with auditors, SLAs, and many sites, that's where the open core ends and **[FleetLink](https://tafylabs.io)** begins — the enterprise layer built on this substrate: multi-tenant gateway, hosted/HA relay, SSO and RBAC, compliance reporting, and fleet-scale OTA, plus fixed-scope architecture reviews and implementations by the people who wrote Ganglion. Start at **[tafylabs.io](https://tafylabs.io)** or email **bobby@tafy.ai**.

## License

Apache-2.0
