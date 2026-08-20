# ADR-026: Telemetry — anonymous, opt-out, operator-side only

- Status: Accepted (implemented; amended in review to add explicit production-disable guidance to the notice, README, and TELEMETRY.md)
- Date: 2026-08-20

## Context

Ganglion has zero telemetry today. The only signals are passive: crates.io
downloads, GitHub traffic, Homebrew's own analytics. Those measure *reach*,
not *use* — they cannot answer "how many live installs exist", "which
versions are in the field", or "does anyone use `gang policy check`".

Ganglion is also a security product whose entire pitch is *provably scoped
egress from inside networks the customer doesn't control*. Telemetry designed
carelessly would contradict the product. This ADR designs it so the pitch and
the telemetry reinforce each other: everything sent is documented, inspectable
before it leaves, aggregated by design, and — above all — **never emitted
from a robot or from inside a customer network**.

## The prime constraint (non-negotiable)

**Telemetry never runs on the robot agent and never inside customer
networks.** Enforced mechanically, in four independent layers, not by policy
prose:

1. **Command allowlist.** Telemetry can only be triggered by an explicit
   allowlist of operator-workstation commands:
   `init, status, deploy, run, caps, peer, policy, registry, sign, view,
   tui, logs, pair, profiles, alert, config, capability`.
   Everything else never touches the telemetry module — in particular
   `agent`, `join`, and `relay` (the long-running processes that run on
   robots and infrastructure), and also `doctor`, `diagnose`, and
   `test-archetype` (field-triage commands frequently run *inside* customer
   networks, where a telemetry attempt would be both a boundary violation
   and measurement noise).
2. **Crate boundary.** The telemetry module lives in `gang-cli` only.
   `gang-core`, `gang-libp2p`, `gang-wasm-host`, and `gang-ros` contain zero
   telemetry code. A workspace test greps those crates' sources for the
   telemetry module path and endpoint host and fails on any reference — the
   same tripwire style as the WIT vendored-copy sync test.
3. **Compile-time feature.** All telemetry code sits behind a `telemetry`
   cargo feature on `gang-cli`. Our release binaries enable it; a
   `--no-default-features` build (distro packagers, hardened deployments)
   contains no telemetry code at all, and `gang telemetry status` then
   reports `compiled out`.
4. **Harness assertion.** The e2e-dispatch harness asserts the robot
   container opens no connection to the checkpoint host during a full
   deploy→invoke round-trip (its compose network makes this cheap to check).

One residual honestly named: an *operator* laptop can itself be inside a
customer network during a site visit. Mitigations: one request per day
maximum, a payload with nothing site-derived (fields below), the standard
`DO_NOT_TRACK` variable, and a single documented endpoint a security team
can block with zero functional impact (blocked = silent no-op, no retry).

## What we build (the four approved components)

### Component A — distribution-side (no client code)

A weekly stats collector (scheduled workflow or vault-side cron; script in
`telemetry/collect-distribution.sh`) snapshots: crates.io downloads per
crate/version (public API), GitHub stars/clones/views (repo API), and
Homebrew installs (Homebrew's public analytics API for the tap formula).
Output: one dated JSON per week, appended to a stats store (repo branch or
vault). Optional later: a Scarf gateway in front of release-binary download
links for company-level install attribution — separate decision, needs a
Scarf account, not part of this ADR's implementation.

### Component B — server-side signals from infrastructure we run

- **Relay:** `gang relay` gains an *operator-opt-in* daily aggregate: unique
  peer count per UTC day, computed against peer IDs hashed with a
  daily-rotating random salt (counts are not linkable across days; raw peer
  IDs never stored). Always written locally (`relay-stats.jsonl`, the relay
  operator's own data); remote reporting to the checkpoint endpoint only
  when the relay config sets `report_stats = true`. **Default off** — a
  self-hosted relay inside a customer network reports nothing. RobotDen's
  hosted relay turns it on.
- **Registry:** publish/install counts are measured by the registry server
  RobotDen already operates — a server-side concern documented here for
  completeness; no client changes.

### Component C — daily checkpoint (update check as the carrier)

At most **once per UTC day**, on the first allowlisted operator command of
the day, the CLI sends one request and prints an update notice when a newer
version exists — the user gets real value from the same request that carries
the telemetry. Mechanics:

- One synchronous request with a 2-second total budget (amended from the
  detached-thread draft: a detached thread can be killed at process exit
  mid-send, making delivery nondeterministic; a bounded synchronous send is
  simpler and honest about its worst case — one command per day may take up
  to 2s longer). **No retries**, failure completely silent, never prints
  errors. The command's exit code and output are byte-identical whether the
  checkpoint succeeds, fails, or is blocked.
- State in `~/.gang/telemetry/`: `last-check` (day stamp), `id` (the
  anonymous id, below), `pending.json` (component D's accumulator).
- Update notice (`gang 2.6.0 available — you have 2.5.0. brew upgrade gang`)
  goes to **stderr**, at most once per new version, and is suppressed
  entirely under `--format json`, `--quiet`, or when stdout is not a TTY.
- Suppressed when the `CI` environment variable is set (pipelines produce
  noise, not usage signal).

### Component D — anonymous usage aggregates (rides the same request)

No per-command pings, ever. Each allowlisted command increments a **local**
counter file: `{command_category: {ok, err}}`. The daily checkpoint flushes
the accumulated counters and resets them. One request per day carries
everything — strictly more private than the per-event Homebrew/Next.js
model (no timing correlation, no event stream).

**The complete payload** (this list is exhaustive; adding a field requires
amending this ADR and `TELEMETRY.md`):

```json
{
  "schema": 1,
  "id": "3f8e…-uuid-v4",          // random; NOT machine- or identity-derived
  "version": "2.5.0",
  "os": "linux", "arch": "aarch64",
  "dist": "brew",                  // brew | binstall | cargo | deb | source
                                   // (baked in at release build; "source" default)
  "counts": { "deploy": {"ok": 3, "err": 1}, "policy": {"ok": 5, "err": 0} }
}
```

Never present, by construction: arguments, robot names, peer IDs, topic or
URL patterns, file paths, policy contents, error text, hostnames, IPs,
locale, timezone, machine identifiers. The command *category* is the
top-level subcommand name only.

**Anonymous id:** UUIDv4 generated lazily at first flush, stored in
`~/.gang/telemetry/id`. Explicitly not derived from the gang identity key,
hardware, or hostname. `gang telemetry reset` regenerates it.

## Consent, transparency, and opt-out

**Notice before first send.** Nothing is ever sent until the disclosure has
been shown once: at `gang init` (new section in its output), or — for
installs that never ran init — in place of the first would-be checkpoint,
which prints the notice and sends nothing that day. The notice states what
is collected, links `TELEMETRY.md`, and shows the one-line opt-out.

**Opt-out layers (any one suffices; checked in this order, all documented):**

| Layer | Mechanism | Audience |
|---|---|---|
| 1 | `DO_NOT_TRACK` env var (any non-empty value) | ecosystem standard |
| 2 | `GANG_TELEMETRY=off` env var | scripts, fleets |
| 3 | `[telemetry] enabled = false` in `~/.gang/config.toml` | persistent per-user |
| 4 | `gang telemetry off` (writes layer 3) | one command |
| 5 | `CI` env var set | pipelines (automatic) |
| 6 | cargo build without the `telemetry` feature | packagers, hardened builds |

**Inspection:** `gang telemetry status` (enabled? by which layer? id? last
send?), `gang telemetry show` (prints the exact pending payload verbatim —
what would be sent, before it is sent), `gang telemetry off|on|reset`.

## Server side (published in-repo for auditability)

`telemetry/worker/` contains the complete checkpoint endpoint — a
Cloudflare Worker — so users can read the receiving side too:

- `POST https://checkpoint.robotden.dev/v1/checkpoint` → responds
  `{ "latest": "2.5.0" }` (latest release, cached from the GitHub Releases
  API for one hour; no release-workflow coupling).
- The worker **discards the request IP** (never stored, never logged),
  validates the schema, drops anything over 4 KiB, and writes one aggregate
  row (date, version, os, arch, dist, category counts, id-hash for
  DAU/WAU dedup — the id is hashed server-side with a server secret so the
  stored value is not the client id). Raw requests are not retained.
  Aggregates retained 13 months.
- Relay daily aggregates (component B) POST to `/v1/relay-stats` with the
  same handling.

## Documentation shipped with the feature

- `TELEMETRY.md` at the repo root: plain-language what/when/why, the
  exhaustive field list, every opt-out, and **verification instructions** —
  `gang telemetry show`, the source paths to read, and a tcpdump one-liner
  proving a `gang agent` robot emits nothing.
- README: one short section linking it.
- `docs/CLI_REFERENCE.md`: the `gang telemetry` command family.
- First-run notice text (verbatim in `TELEMETRY.md` so docs and binary
  can't drift).
- `CHANGELOG.md` entry marking the release that introduces it.

## Rollout

1. **Phase 1 (client, dark):** telemetry module, `gang telemetry` commands,
   local accumulation, notice wiring, crate-boundary guard test, harness
   assertion — with sending compiled in but pointing at the endpoint that
   does not exist yet; silent-failure semantics make this a safe soak.
2. **Phase 2 (endpoint):** deploy the worker (Bobby: Cloudflare account +
   `checkpoint.robotden.dev` DNS), verify aggregates, then announce in the
   release notes of the version that turns the default on.
3. **Phase 3 (periphery):** relay opt-in aggregates; distribution stats
   collector; Scarf decision deferred.

## Acceptance criteria

- A robot running `gang agent`/`gang join` through the full e2e harness
  makes zero connections to the checkpoint host (asserted in harness).
- `rg telemetry crates/gang-{core,ros,libp2p,wasm-host}` is empty (guard
  test).
- With the endpoint unreachable/blocked, every CLI command's output, exit
  code, and latency (±2s worst case on one command per day) are unchanged.
- `gang telemetry show` output matches byte-for-byte what the wire would
  carry; the field list matches this ADR and `TELEMETRY.md`.
- Every opt-out layer independently results in zero requests (tested).
- No payload field beyond the exhaustive list above; schema version bump +
  ADR amendment required to add one.

## Rejected alternatives

- **Per-command event pings (Homebrew/Next.js model):** more granular, but
  creates an event stream correlatable by timing; the daily aggregate loses
  almost nothing we care about and is materially more private.
- **Machine-derived IDs** (MAC/hostname hashes): needless deanonymization
  risk; a random UUID dedups DAU/WAU just as well and is user-resettable.
- **Third-party analytics SDK (PostHog/Segment):** a data-processor
  relationship and GDPR surface that reads wrong for this product; our
  payload is small enough that a worker + aggregates is less total code.
- **Telemetry in the robot agent** ("fleets are the interesting data"):
  categorically excluded — it contradicts the product's core claim. Fleet
  visibility comes from component B on infrastructure *we* run, where
  observation is inherent and documented.
- **Opt-in instead of opt-out:** opt-in yields single-digit-percent
  response rates and self-selected samples; with a payload this minimal,
  first-send-after-notice plus six opt-out layers is the honest middle.
