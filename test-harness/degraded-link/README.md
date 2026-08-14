# Degraded-link matrix (#32)

Ganglion's claim is reachability over networks you don't control — so CI must
exercise exactly the conditions a clean runner never produces. This matrix
runs the **real e2e-dispatch round-trip** (deploy → invoke → verify over the
relay circuit) under five link profiles, with impairment applied inside the
containers *before* the agent starts.

## Profiles

| Profile | Impairment | Determinism |
|---|---|---|
| `clean` | none (baseline) | deterministic |
| `lossy` | 3% loss (**every 33rd packet**, iptables statistic-nth) + fixed 40ms delay | deterministic |
| `high-latency` | 250ms RTT (125ms each way), zero jitter | deterministic |
| `asymmetric` | robot uplink capped 192kbit + 30ms; downlink clean | deterministic |
| `nat-relay` | direct robot↔operator path firewalled; relay circuit only | deterministic |

## Usage

```bash
./run-matrix.sh                      # required gate: all 5 profiles
./run-matrix.sh --profile asymmetric # one profile
./run-matrix.sh --profile-file site.profile  # external fixture (see below)
./run-matrix.sh --chaos              # randomized netem (nightly), seed recorded
./run-matrix.sh --chaos --seed 42    # replay a chaos run's impairment
```

## Determinism contract

The **gate** profiles use only mechanisms that reproduce exactly run-to-run:
fixed netem delay, tbf rate caps, iptables `statistic --mode nth` loss, and
route blocking. netem's *random* loss/jitter/reorder distributions are
reserved for **chaos** mode, because netem's per-packet draw comes from
kernel RNG and cannot be seeded: a chaos replay (`--seed N`) reproduces the
exact impairment *parameters* — the distribution — not the packet-level
trace. That is the honest limit of netem replay, and it is why chaos runs
never block merges.

## Artifacts

Every run writes `artifacts/<timestamp>-<profile>.json` recording the mode,
seed, exact shaping commands, duration, and result — a failing run carries
its own replay instructions.

## CI wiring

Ready-to-install workflow pieces live in `workflows/` (they must be moved
into `.github/workflows/` by a committer whose token has workflow scope):

- `ci-degraded-link-job.yml` — the required-gate job to append to `ci.yml`
  (main-push only, single job, one build, sequential profiles, early-exit on
  docs-only pushes).
- `nightly-chaos.yml` — daily randomized run; non-blocking; opens an issue
  with the seed and artifact on failure.

## Site profiles from the field (#33)

`gang doctor --profile-out site.profile` measures a customer link (RTT median
and connect-failure rate against the configured relay) and emits a fixture in
exactly this directory's format — fixed netem delay + statistic-nth loss, with
the measurement provenance in header comments and operator-supplied rate caps
via `--uplink-kbit`/`--downlink-kbit`. Replay the customer's network any time:

```bash
./run-matrix.sh --profile-file acme-east.profile
```

Site profiles carry `PROFILE_CLASS="site"` (no gate retry semantics — they run
once like chaos, but with deterministic shapes). Check the ones worth keeping
into `profiles/` to make a customer link part of the permanent matrix.
