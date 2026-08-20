# Telemetry

Ganglion's CLI collects **anonymous, aggregate usage data** to help us build
better tools — which commands get used, on which platforms, on which
versions. That's the entire purpose. **Telemetry is enabled by default**
(the same opt-out model as Homebrew, Next.js, and the HashiCorp tools);
every way to turn it off is documented below, and nothing is ever sent
before the one-time disclosure notice has been shown. This page is the
complete story: what is sent, when, what never is, and every way to turn
it off.

> **Production guidance — read this first.**
> Telemetry never runs from `gang agent`, `gang join`, or `gang relay` — the
> processes that live on robots and infrastructure — and never from the
> field-triage commands (`gang doctor`, `gang diagnose`, `gang
> test-archetype`). That boundary is enforced in code, tests, and the CI
> harness, not just policy.
> **Even so: telemetry should not run on your robots or in your customers'
> environments. If you operate Ganglion in production, disable it outright —
> run `gang telemetry off` on every operator workstation, or set
> `DO_NOT_TRACK=1` fleet-wide.** Disabling changes nothing about how
> Ganglion works.
>
> Robots themselves need nothing disabled to be safe: the agent's local
> usage bundle (see below) has **no send path at all** — it is a local file,
> and only an operator's explicit opt-in ever forwards an aggregate of it.

## How to disable it (any ONE of these)

| Method | Scope |
|---|---|
| `gang telemetry off` | this machine, persistent (writes config.toml) |
| `export DO_NOT_TRACK=1` | ecosystem-standard env var |
| `export GANG_TELEMETRY=off` | Ganglion-specific env var |
| `[telemetry]` `enabled = false` in `~/.gang/config.toml` | what `gang telemetry off` writes |
| `CI` env var set | automatic in pipelines |
| build with `--no-default-features` | compiles telemetry out entirely (packagers) |

`gang telemetry status` tells you whether telemetry is enabled and, if not,
exactly which layer disabled it.

## What is sent, exactly

At most **one request per day**, from operator commands only, after a
one-time disclosure notice (nothing is sent before the notice has been
shown). Commands increment a local counter; the first operator command of a
UTC day flushes the aggregate. There is no event stream and no per-command
ping. The same request answers with the latest release version, which is how
`gang` can tell you an update exists.

The complete payload — this list is exhaustive, and `gang telemetry show`
prints yours byte-for-byte before anything is sent:

```json
{
  "schema": 1,
  "id": "8b1e…",                    // random UUID — see "Anonymity" below
  "version": "2.5.0",
  "os": "linux",
  "arch": "aarch64",
  "dist": "brew",
  "counts": { "deploy": {"ok": 3, "err": 1}, "policy": {"ok": 5, "err": 0} }
}
```

**Never sent, by construction:** command arguments, robot or peer names,
topic/URL/file patterns, policy contents, error messages, hostnames, IP
addresses (the server discards them — see below), locale, timezone, or any
machine identifier.

## Anonymity

The `id` is a random UUID generated locally on first use. It is **not**
derived from your machine, hostname, MAC address, or your gang identity
key. `gang telemetry reset` regenerates it at any time. Server-side it is
hashed with a secret before storage, so even the stored value can't be
matched back to the id on your disk.

## Transparency: the whole pipeline is in this repository

- Client: [`crates/gang-cli/src/telemetry.rs`](crates/gang-cli/src/telemetry.rs)
  — the allowlist, the payload struct, the opt-out checks, the notice text
  (the notice shown by the binary is asserted against this file's copy by a
  test, so they cannot drift).
- Server: [`telemetry/worker/`](telemetry/worker/) — the complete
  Cloudflare Worker: request logging disabled (no IPs at rest), 4 KiB cap,
  schema validation, server-side id hashing, aggregate rows only, 13-month
  retention.
- Design: [`docs/adr/ADR-026-telemetry.md`](docs/adr/ADR-026-telemetry.md).
- Distribution stats (no client code at all):
  [`telemetry/collect-distribution.sh`](telemetry/collect-distribution.sh).

## Verifying the boundary yourself

- `gang telemetry show` — the exact payload, before it is sent.
- The command allowlist is a `const` at the top of `telemetry.rs`; `agent`,
  `join`, `relay`, `doctor`, `diagnose`, and `test-archetype` are absent,
  and a unit test asserts they stay absent.
- A workspace test fails the build if `gang-core`, `gang-ros`,
  `gang-libp2p`, or `gang-wasm-host` ever reference the telemetry module or
  endpoint.
- The e2e harness fails if the checkpoint host appears anywhere in a robot
  container's traffic logs.
- On your own robot: `sudo tcpdump -n host checkpoint.robotden.dev` while
  `gang agent` runs — you will see nothing, ever.
- Network-level: blocking `checkpoint.robotden.dev` breaks nothing; the CLI
  treats it as a silent no-op with no retries.

## The first-run notice (verbatim)

```text
Ganglion telemetry notice (shown once)
--------------------------------------
To help us build better tools, the gang CLI sends ONE anonymous request per
day from operator commands: a random id (no machine or identity data), the
CLI version, OS/arch, install channel, and per-command success/error counts.
Never arguments, names, patterns, peers, URLs, or anything from your network.
Inspect exactly what would be sent:   gang telemetry show
Disable with any of:                  gang telemetry off
                                      export DO_NOT_TRACK=1
The full story, field list, and every opt-out: TELEMETRY.md in the repo.

  Telemetry never runs from `gang agent`, `gang join`, or `gang relay` —
  but if you operate Ganglion in production, on robots or in customer
  environments, DISABLE IT OUTRIGHT: `gang telemetry off` on every
  operator workstation. Nothing was sent today; sending starts tomorrow.
```

## Fleet usage bundles (local-only on robots; forwarding is opt-in)

Robots running `gang agent` accumulate a **usage bundle**: a local JSON
file of anonymous, ganglion-usage-only counters — per-capability-**group**
ok/err counts (`ros`, `fs`, `diagnostics`, …) and a bare policy-denial
count. **The robot never transmits it.** There is no send path in the
robot's code (a guard test keeps it that way), so the transmission boundary
above is unchanged: nothing leaves a robot or a customer network on its
own, ever.

The complete bundle — this list is exhaustive:

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

`errors` breaks failures out by kind from a **closed set the runtime
defines** (`trapped`, `deadline`, `policy-denied`, `fuel-exhausted`,
`hash-mismatch`, `failed`) — never error messages or free text.

**Never in a bundle, by construction:** capability *names* (operator-named
capabilities can identify customers or sites — categories, never names),
robot or peer identifiers, topics, patterns, paths, policy contents,
arguments, error text, hostnames, or IPs.

Operators may fetch bundles over the same authenticated channel used for
deploys — `gang telemetry fleet pull` — into a **local** accumulator, and
nothing goes further unless the operator explicitly runs
`gang telemetry fleet on` (default: **off**). When on, the daily checkpoint
also sends one aggregated fleet payload: bucketed robot count ("2-5",
"6-20", …), counts **summed across the fleet** (per-robot rows never leave
the operator's machine), the set of agent versions, and the total denial
count. `gang telemetry fleet show` prints the exact payload beforehand.
Every opt-out above also disables fleet forwarding.

Robot-side controls: set `DO_NOT_TRACK=1` or `GANG_TELEMETRY=off` in the
agent's environment (e.g. in its systemd unit) to stop accumulation, or
build `gang-ros` without the `usage-bundle` feature to compile it out.
Design: ADR-027.

## Relay operators (opt-in only)

Self-hosted relays report **nothing**. A relay operator may opt in to
sending a daily unique-peer count (peer IDs hashed with a daily-rotating
salt, unlinkable across days); RobotDen's hosted relay does. See ADR-026.
