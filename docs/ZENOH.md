# Running Ganglion on rmw_zenoh fleets

Zenoh is the transport/discovery layer more and more ROS 2 fleets are moving to
(`rmw_zenoh` is a Tier-1 RMW as of ROS 2 Jazzy/Kilted). Ganglion is designed to
ride that wave, not fight it. Zenoh answers *how do topics and services move
between nodes*; it does not answer *who is the remote operator, what may they
run, and what did they do* — and its security is off by default. Those last
questions are exactly Ganglion's job. "Ganglion runs great on Zenoh fleets" is a
statement about composition, not competition.

## Why it just works

Ganglion's ROS layer (`gang-ros`) talks to the robot's ROS graph through the
standard `ros2` command-line tooling — `ros2 topic list`, `ros2 topic info -v`,
`ros2 node list`, `ros2 service list`, and friends — invoked directly (no shell)
with capability-scoped pattern checks in front of every call. That tooling is
**RMW-agnostic**: it uses whatever `RMW_IMPLEMENTATION` the environment selects.
Point it at `rmw_zenoh_cpp` and the same broker code drives a Zenoh graph with
no changes:

```bash
export RMW_IMPLEMENTATION=rmw_zenoh_cpp
# start the zenoh router (rmw_zenoh's `ros2 run rmw_zenoh_cpp rmw_zenohd`)
gang agent            # the Ganglion robot agent — brokers now speak to a Zenoh graph
```

Because the brokers never link a specific middleware and never assume DDS
discovery semantics, there is no DDS-only code path to port. The capability
model, signed-manifest verification, default-deny policy, and audit log are all
middleware-independent.

## The division of labor

| Concern | Zenoh (`rmw_zenoh`) | Ganglion |
|---|---|---|
| Move topics/services between nodes | ✅ | — |
| Discovery, WAN routing (zenoh-bridge) | ✅ | — |
| Reach a robot behind a hostile firewall from outside | partial (needs a routed zenoh-bridge you operate) | ✅ outbound-only relay + DCUtR |
| Operator identity on the wire | ❌ (off by default) | ✅ Ed25519, authenticated end-to-end |
| What tooling may run, scoped to declared capabilities | ❌ | ✅ signed WASM + default-deny policy |
| Audit trail of every remote action | ❌ | ✅ append-only, hash-chained |

Zenoh gets data around the fleet; Ganglion makes *reaching in from outside* and
*acting on the robot* safe and accountable. They stack cleanly.

## Zenoh security is opt-in — Ganglion assumes hostile

Zenoh can be secured (TLS, access-control config on the router), but it ships
insecure by default and the security posture is the operator's to assemble.
Ganglion inverts that default: identity, policy, and audit are always on, and
the network is assumed hostile. Deploying Ganglion on a Zenoh fleet therefore
adds the operator-facing security story Zenoh leaves to you, without displacing
Zenoh's role on the robot and LAN.

## Status and validation

The compatibility above follows from the RMW-agnostic `ros2`-CLI integration and
is documented as such. End-to-end validation against a live `rmw_zenoh_cpp`
graph (a CI rig that boots a zenoh router and exercises the brokers) is tracked
separately; if you run Ganglion on a Zenoh fleet, `gang diagnose` and the ROS
brokers should behave identically to a DDS fleet — please open an issue with
`gang --version`, your ROS distro, and `RMW_IMPLEMENTATION` if anything differs.
