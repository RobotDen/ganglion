# When to Use Ganglion (and When Not To)

Most tools in this space answer one question: *can I reach the robot?* Ganglion
answers two: *can I reach it, **and** can I guarantee that the only thing that
runs on it is signed, sandboxed, policy-bounded, audited tooling?*

That framing sorts the landscape cleanly:

- **Network planes** (Tailscale, WireGuard, Husarnet, OpenZiti) get packets to
  the robot. What happens after the connection is up is out of scope.
- **Data planes** (DDS, Zenoh) move topics, services, and queries — inside the
  robot, across the LAN, and (with routers/bridges) across the WAN.
- **Ganglion is a governed execution substrate** that happens to bring its own
  hostile-network connectivity (libp2p relay + DCUtR, outbound-only). It
  composes *with* DDS or Zenoh on the robot; it does not replace them.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the three-layer design and
[SECURITY.md](SECURITY.md) for the threat model this document keeps referring
back to.

## VPNs: Tailscale, WireGuard, and friends

**What they are.** Mesh/point-to-point VPNs that put your robots and laptops on
a shared virtual network. Mature, fast, widely deployed.

**Choose a VPN when** you control both ends of the network, your team is small,
and "SSH into the robot" is an acceptable operational model.

**Choose Ganglion when** the robot lives in a customer's network you can't
configure, or when "anyone on the VPN can run anything as root" is the risk
you're trying to eliminate. A VPN authenticates *machines*; once connected, a
shell has ambient authority. Ganglion authenticates *every request*, refuses
anything unsigned, sandboxes what runs, and writes a tamper-evident audit
trail ([SECURITY.md](SECURITY.md)).

**Use both** if you already run a VPN for general access: Ganglion's transport
works fine over it, and you keep the execution governance.

## Husarnet

**What it is.** A [peer-to-peer VPN built for ROS](https://husarnet.com/robotics)
— low-latency P2P paths, DDS-friendly, popular for teleop and multi-robot ROS 2
over the internet.

**Choose Husarnet when** you want ROS 2 traffic itself (topics, teleop streams)
flowing between sites with minimal setup, and network-layer access control is
sufficient.

**Choose Ganglion when** the problem is field *tooling*, not topic transport:
diagnostics, log capture, parameter inspection on robots you don't own the
network for — with per-tool capability grants instead of network membership.

**Use both**: Husarnet (or any VPN) for live topic streams; Ganglion for the
governed run-a-tool-and-get-an-artifact workflows.

## OpenZiti

**What it is.** An open-source [zero-trust overlay network](https://openziti.io)
— outbound-only tunnels, identity-based service access, app-embeddable SDKs.
Philosophically the closest network plane to Ganglion: it also assumes hostile
networks and denies by default.

**Choose OpenZiti when** you need zero-trust access to many kinds of *services*
(dashboards, SSH, APIs) across an organization, and you're prepared to run its
controller/router infrastructure.

**Choose Ganglion when** the unit you need to govern is a *tool execution*, not
a network flow. OpenZiti decides who may open a connection to a service; it has
no opinion about what code runs once connected. Ganglion's unit of authorization
is "this signed WASM component, with these declared capabilities, against these
ROS topic patterns" — enforced by brokers, recorded in a hash-chained audit log.

## Zenoh

**What it is.** [Eclipse Zenoh](https://zenoh.io) is a pub/sub/query protocol
that unifies data in motion and at rest, with a router for wide-area topologies,
`zenoh-pico` for microcontrollers, a DDS bridge, and first-class ROS 2 support
via [`rmw_zenoh`](https://github.com/ros2/rmw_zenoh) (a Tier-1 RMW since ROS 2
Kilted). It is excellent at what it does: scalable, WAN-capable data flow
without DDS discovery storms, plus TLS, mutual auth, and router-level ACLs.

**Choose Zenoh when** your problem is *data distribution* — telemetry across
sites, DDS-over-WAN, edge-to-cloud dataflow, constrained devices.

**Choose Ganglion when** your problem is *governed action*. Zenoh's ACLs gate
who may publish/subscribe to which keys; they do not sign code, sandbox
execution, meter CPU/memory, or produce a per-request tamper-evident audit
trail. "Run this diagnostic bundle on robot X, bounded by policy, and prove
later exactly what ran" is not a dataspace problem.

**Use both** — this is the expected production shape: `rmw_zenoh` or a Zenoh
router as the robot's data plane, Ganglion as the execution plane for signed
field tooling. They share a robot without conflict; Ganglion's ROS broker sits
on top of whatever RMW the robot runs.

## DDS (Fast DDS, Cyclone DDS) and WAN routers

**What it is.** The ROS 2 default middleware: real-time pub/sub with rich QoS,
designed for in-robot and LAN communication. For WAN, the
[eProsima DDS Router](https://eprosima-dds-router.readthedocs.io/en/latest/)
or the Zenoh bridge extend DDS across sites over TCP.

**Choose DDS when** — you already did; it ships with ROS 2 and is the right
answer for intra-robot and LAN real-time messaging.

**Choose Ganglion when** you hit DDS's hostile-network edges: multicast
discovery doesn't cross NAT, symmetric NAT and CGNAT defeat peer-to-peer
entirely, and enterprise DMZs allow outbound 443 and nothing else. A DDS Router
helps with transport but still typically needs an addressable endpoint, and —
like all of DDS — has no execution model: it moves samples, it doesn't run,
sign, or audit tools.

**Use both**, always: Ganglion never replaces DDS on the robot. Its brokers
speak to the local ROS 2 graph; its libp2p layer carries requests and artifacts
where DDS can't reach.

## PyZeROS

**What it is.** An experimental, pure-Python alternative to `rclpy` that talks
to ROS 2 over Zenoh ([github.com/2lian/pyzeros2](https://github.com/2lian/pyzeros2)) —
pip-installable, no ROS installation needed, asyncio-native. A developer
ergonomics project, and a neat one.

**Choose PyZeROS when** you want to write Python clients against a ROS 2 graph
without a colcon workspace, and you're comfortable with experimental APIs.

**Choose Ganglion when** the question is not "how do I write a ROS client" but
"how do I let *someone else's laptop* run bounded tooling against a robot in a
network I don't control." PyZeROS inherits whatever network reachability and
security Zenoh gives it; it has no signing, sandboxing, policy, or audit story —
those were never its goals.

## Observability and fleet platforms

- **[Foxglove](https://foxglove.dev)** — robotics observability: visualization,
  MCAP data management, dashboards. Superb at *seeing* what robots did (the
  platform, including the agent, is a commercial product as of Foxglove 2.0,
  2024). It is not an execution substrate. Use Foxglove to visualize the data
  Ganglion tools capture; there's no overlap to resolve.
- **[Viam](https://www.viam.com)** — a full app platform/SDK for building robot
  behavior, with cloud management. Choose it if you want a managed platform to
  *build* robots on. Ganglion assumes your stack is ROS 2 and adds a governed
  tooling layer; it doesn't ask you to re-platform.
- **[Transitive Robotics](https://transitiverobotics.com)** — open-core
  full-stack robot web capabilities (remote teleop, video). Closest in spirit
  among the platforms; its trust model is capability packages over MQTT, not
  signed sandboxed WASM with default-deny brokers.
- **Formant** — historically the reference "robot data + teleop platform";
  it has since pivoted to AI incident management for physical operations, which
  says something about how hard the managed-robotics-SaaS market is.

## Ganglion is NOT for you if…

- **Your robots live on one site, on a LAN you own, and SSH works.** Keep SSH.
  Ganglion earns its complexity on networks you *don't* control.
- **You want a managed SaaS with dashboards and support.** Ganglion is an
  Apache-2.0 substrate you operate yourself. (That said — the commercial layer
  on top of it is [FleetLink](https://tafylabs.io).)
- **You don't run ROS 2.** The connectivity and WASM layers are generic, but
  the brokers, capabilities, and tooling are built for ROS 2 fleets. Elsewhere
  you'd be using half the project.
- **You need hard real-time remote control.** Ganglion is for tooling, not for
  closing 1 kHz control loops over a relay.

## Decision guide

1. **Can you already reach the robot, and do you trust everyone who can?**
   Yes → SSH/VPN is fine. No → keep reading.
2. **Is the problem moving topic data across sites?** → Zenoh (or DDS Router),
   possibly over Husarnet/Tailscale.
3. **Is the problem running tools on robots inside customer networks, with
   proof of exactly what ran?** → Ganglion.
4. **Both?** → Zenoh/DDS for telemetry, Ganglion for governed tooling. They
   compose; that's the point.

For how the guarantees are actually enforced, read
[SECURITY.md](SECURITY.md); for where each piece lives,
[ARCHITECTURE.md](ARCHITECTURE.md); for which network environments were
designed for, [NETWORK_ARCHETYPES.md](NETWORK_ARCHETYPES.md).
