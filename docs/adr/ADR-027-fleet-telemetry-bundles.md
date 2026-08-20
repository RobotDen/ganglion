# ADR-027: Fleet telemetry bundles — local-only robot counters, operator-mediated forwarding

- Status: Accepted (implemented)
- Date: 2026-08-20
- Depends on: ADR-026 (telemetry), ADR-024 (event push stream)

## Context

ADR-026 deliberately excluded robots from telemetry: the checkpoint runs only
on operator workstations, and four mechanical layers keep telemetry code and
traffic away from `gang agent`. That gives us workstation usage data but
leaves the most interesting question unanswered: **what do deployed fleets
actually use?** Which capability groups run in the field, at what error
rates, on which agent versions — the data that would most directly shape
what we build next.

ADR-026's answer was "component B: observe from infrastructure *we* run."
That covers RobotDen's hosted relay, but most production fleets in the
regulated/enterprise archetypes never touch our infrastructure at all.

The insight behind this ADR: the constraint was never "robots must have no
usage counters." It was **"telemetry must never transmit from a robot or
from inside a customer network."** A robot can accumulate anonymous,
ganglion-usage-only counters into a local file — exactly as it already
accumulates an audit log — without ever opening a connection. The existing
Ganglion control channel (which the operator already uses to deploy and
invoke capabilities) can carry that file to the operator on request. The
operator's workstation — the one place telemetry already runs — then decides
whether anything leaves.

This softens the "disable it outright in production" posture without moving
the transmission boundary an inch: **the robot still never transmits
telemetry, ever.** What changes is that "telemetry off on robots" stops
being the only safe production configuration, because the robot-side
mechanism is local-only by construction.

## Decision

Three pieces, strictly layered so each can be disabled independently and
the transmission boundary is enforced by the same mechanisms ADR-026
already ships.

### 1. Robot-side: the usage bundle (local file, never transmitted)

`gang agent` accumulates a **usage bundle**: a small JSON file in the agent
data directory (`usage-bundle.json` next to the audit log; the path is
`AgentConfig::usage_bundle_path`), updated as capabilities run. It is the telemetry analogue of the audit log — written locally,
readable by the operator, never pushed anywhere by the robot.

**The complete bundle** (exhaustive; adding a field requires amending this
ADR and `TELEMETRY.md`):

```json
{
  "schema": 1,
  "version": "2.6.0",
  "os": "linux",
  "arch": "aarch64",
  "counts": { "ros": {"ok": 41, "err": 2}, "fs": {"ok": 7, "err": 0} },
  "errors": { "ros": {"trapped": 1, "deadline": 1} },
  "denials": 3
}
```

- `errors` breaks the `err` totals out by failure kind — a **closed set
  defined by the runtime** (`trapped`, `deadline`, `policy-denied`,
  `fuel-exhausted`, `hash-mismatch`, `failed`). Never messages, never free
  text; an unknown kind degrades to `failed` (test-enforced).
- `counts` is keyed by **capability group** (`ros`, `logs`, `fs`,
  `diagnostics`, `artifacts`, `process`, `network`, `metrics`, `http`) —
  the WIT interface families, a closed set defined by us. **Never**
  capability names: operators write custom capabilities whose names can
  identify customers, sites, or use cases (`acme-line3-plc-probe`).
  Categories, never names.
- `denials` is a bare count of policy denials — no patterns, no operations.
- **No identifier of any kind.** Not even a random one. Individual robots
  are never distinguishable in anything that leaves the operator's machine
  (see §3), so the bundle itself carries nothing to link.
- Never present, by construction: robot/peer names, peer IDs, topics,
  patterns, paths, policy contents, arguments, error text, hostnames, IPs,
  uptime, timestamps beyond the file's own mtime.

**Robot-side controls.** Accumulation is on by default (it is a local file
with less information in it than the audit log the agent already writes),
and independently disableable: `DO_NOT_TRACK` or `GANG_TELEMETRY=off` in
the agent's environment (e.g. its systemd unit) disables accumulation, a
`None` bundle path in `AgentConfig` disables it for embedders, and building
`gang-ros` without the `usage-bundle` feature compiles it out.

**The boundary, restated for the new model.** ADR-026's prime constraint
becomes, precisely: **telemetry never *transmits* from a robot or relay.**
The enforcement layers are unchanged in kind:

1. The robot agent contains no telemetry endpoint, no HTTP client for it,
   and no send path. The bundle module (`usage_bundle` in `gang-ros`) can
   only write a local file.
2. The guard test that greps robot-side crates for the checkpoint host and
   the CLI telemetry module stays, verbatim. `usage_bundle` never
   references either.
3. The e2e harness assertion stays: a robot container makes zero
   connections to the checkpoint host through a full deploy→invoke
   round-trip — now *with bundle accumulation enabled*, which turns the
   assertion from "robots have no telemetry code" into the stronger,
   correct claim "robots with usage counters still never transmit."
4. The relay gets no bundle and no fetch role (rejected below).

### 2. Transport: operator-mediated fetch over the existing control channel

A new additive `ControlMessage` pair: `FetchUsageBundle` →
`UsageBundleReport { bundle_json }`. Same channel, same encryption, same
peer authentication as every other agent RPC. An agent with bundles
disabled (or predating this feature) answers with an empty report / error,
which the CLI treats as "no data" — never a failure.

`gang telemetry fleet pull [peer…]` fetches bundles from reachable robots
and merges them into the operator's **local** fleet accumulator
(`~/.gang/telemetry/fleet.json`). Pulling is always allowed when the
operator can reach the robot — it moves data from a machine the operator
administers to the operator's own laptop, the same trust step as `gang
logs` or artifact download. Pulled bundles are also **reset** robot-side on
successful fetch (counts are deltas, so double-counting is impossible).

**No relay variant.** Bobby's original sketch allowed a relay to do the
daily push. Rejected: a relay is infrastructure that frequently sits inside
customer networks, and giving it a telemetry send path would re-cross the
transmission boundary we just defended, complicate the guard tests, and
save one manual step at most. Operator-mediated only.

### 3. Forwarding: explicit opt-in, aggregated, riding the daily checkpoint

Nothing pulled ever leaves the operator's machine unless the operator has
run **`gang telemetry fleet on`** — a separate, explicit opt-in, default
**off**, independent of (and additionally gated by) the ADR-026 workstation
telemetry disposition. Every existing opt-out layer (`DO_NOT_TRACK`,
`GANG_TELEMETRY=off`, `CI`, config, compiled-out feature) also disables
fleet forwarding: fleet forwarding is strictly a subset of workstation
telemetry, never an extra channel.

When enabled, the daily checkpoint send is followed by one POST to
`/v1/fleet` with **the complete fleet payload** (exhaustive):

```json
{
  "schema": 1,
  "id": "3f8e…",                    // the operator's ADR-026 anonymous id
  "version": "2.6.0",               // operator CLI version
  "robots": "2-5",                  // bucket: "1" | "2-5" | "6-20" | "21-100" | "100+"
  "agent_versions": ["2.5.0", "2.6.0"],   // unique, sorted; no counts
  "counts": { "ros": {"ok": 412, "err": 9} },   // summed across all robots
  "errors": { "ros": {"trapped": 4, "deadline": 5} },  // same closed kind set
  "denials": 14
}
```

- Counts are **summed across the fleet before sending**; per-robot rows
  never leave the operator's machine. Robot count is bucketed so small
  fleets aren't fingerprintable. Agent versions are a deduplicated set —
  no per-version robot counts, for the same reason.
- Same wire discipline as the checkpoint: at most once per UTC day, 2s
  budget, no retries, byte-identical CLI behavior on failure, silent
  no-op when blocked.
- `gang telemetry fleet show` prints the exact pending fleet payload
  byte-for-byte, mirroring `gang telemetry show`.
- Server side: `/v1/fleet` added to the in-repo worker with the same
  guarantees — exhaustive key validation, 4 KiB cap, IP discarded,
  server-side id hashing, aggregate rows only, 13-month retention.

## What this changes about the production guidance

`TELEMETRY.md`'s production message stays, sharpened rather than softened:

- Robots: **nothing to disable to stay safe** — the robot never transmits,
  with or without bundles. Operators who want zero local counters can still
  set `[telemetry] bundle = false` per agent or compile it out.
- Customer-network workstations: guidance unchanged — disable telemetry on
  workstations that operate inside customer environments.
- Fleet forwarding: off until you say otherwise, and it only ever carries
  ganglion usage categories — nothing about the customer, the site, or the
  robots' work.

## CLI surface

```
gang telemetry fleet status        # forwarding on/off, robots pulled, last send
gang telemetry fleet on|off       # explicit opt-in / opt-out for forwarding
gang telemetry fleet pull [peer]  # fetch + merge bundles (all known peers by default)
gang telemetry fleet show         # exact pending /v1/fleet payload
gang telemetry fleet reset        # clear the local fleet accumulator
```

`gang telemetry status` gains one line reporting fleet forwarding state.

## Acceptance criteria

- e2e harness passes with bundle accumulation **enabled** on the robot and
  still records zero robot connections to the checkpoint host.
- Guard tests: robot-side crates never reference the endpoint host or the
  CLI telemetry module; the bundle payload and fleet payload field lists
  are locked by tests mirroring the ADR-026 payload-lock test.
- With fleet forwarding off (default), `/v1/fleet` is never contacted even
  when telemetry is otherwise enabled and bundles have been pulled.
- Every ADR-026 opt-out layer independently disables fleet forwarding.
- `gang telemetry fleet show` matches the wire bytes.
- Pull-then-pull yields no double counting (robot-side reset on fetch).
- A capability name never appears in a bundle, fleet accumulator, or fleet
  payload (test constructs a capability with a distinctive name and greps
  all three).

## Rejected alternatives

- **Relay-mediated daily push** (from the original idea): re-crosses the
  transmission boundary from inside customer networks; operator-mediated
  loses only convenience, not data.
- **Per-capability-name counts:** names are operator-authored and can
  encode customers/sites; categories answer the product question ("which
  capability *families* earn their keep") without the identification risk.
- **Per-robot rows with random ids:** even random ids allow fleet-shape
  fingerprinting over time; summing before sending costs us per-robot error
  attribution we don't need.
- **Bundling into the checkpoint payload:** would break the ADR-026
  exhaustive-payload lock and couple two consent decisions (workstation
  telemetry vs fleet forwarding) that must stay independent.
- **Robot-initiated push on a schedule:** categorically excluded; it is the
  exact thing the product promises never happens.
