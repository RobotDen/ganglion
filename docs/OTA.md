# OTA updates: pair Ganglion with Mender or balena

Ganglion does not do over-the-air OS or application updates, and it is not going
to. Fleet OTA is a mature, well-served problem — Mender and balena both do it
well — and the failure mode of this product category is trying to become the
whole platform. Ganglion stays the substrate: outbound-only reachability plus
signed, sandboxed, audited *tooling*. This page is the buy-vs-build answer for
teams who need both.

## Who does what

| Responsibility | Ganglion | Mender / balena |
|---|---|---|
| Reach a robot behind a customer firewall (outbound-only) | ✅ | — |
| Run signed, capability-scoped tooling on the robot, audited | ✅ | — |
| Live diagnostics / debugging into a deployed fleet | ✅ | — |
| Roll out a new OS image or app container, with staged rollout + rollback | — | ✅ |
| Fleet-wide update campaigns, A/B partitions, delta updates | — | ✅ |

The line is clean: **Ganglion is how you get in and act safely; Mender/balena
is how you push new bits.** They do not overlap, and neither needs the other's
mechanism.

## Reference layout

A typical robot runs both, independently:

- **balena** (or **Mender**) owns the OS/container lifecycle. The `gang` agent is
  just another managed component in the image — installed via the one-line
  installer (`scripts/install.sh`) baked into the image build, or shipped as a
  balena service / Mender-managed package.
- **Ganglion** owns remote reach + tooling. It dials out to your relay
  (outbound-only), so it needs no inbound ports and no coordination with the OTA
  vendor's connectivity.

```
robot
├── OS + app containers ......... managed by Mender / balena (OTA)
└── gang agent .................. managed by Ganglion (reach + tooling)
     └── dials out to your relay (no inbound ports)
```

Because both are outbound-only, they coexist behind the same hostile firewall
without competing for inbound access. A `gang doctor` run tells you exactly what
egress each needs (see [CLI_REFERENCE](CLI_REFERENCE.md#gang-doctor)).

## When the update itself needs debugging

The natural handoff: when an OTA rollout misbehaves on a specific robot — a
service won't come up, a config is wrong, a sensor is silent after the update —
that is a *reach + tooling* problem, which is Ganglion's half. Use Mender/balena
to ship the fix once you know what it is; use Ganglion (`gang view`, a
capability-scoped diagnostic, the audit log) to find out what it is. Don't make
Ganglion push images, and don't make your OTA vendor grow a ROS debugger.
