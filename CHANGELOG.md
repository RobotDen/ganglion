# Changelog

All notable changes to Ganglion will be documented in this file.

## [Unreleased]

## [2.5.0] - 2026-08-20

### Added

- **`ganglion:http/egress` — the ninth capability group (ADR-025).**
  URL-pattern-allowlisted outbound HTTP for API-integration capabilities:
  endpoints declared as URL globs with read_only (GET/HEAD) or read_write
  access, gated by the policy engine like every other group, re-validated
  per call against the component's own declaration (query strings stripped),
  with host-side TLS, a 256 KiB response cap that errors rather than
  truncates, a 10 s deadline, redirects returned as data (never followed),
  and transport-owned headers refused. Path- and method-scoped — strictly
  stronger than address allowlists. Closes #41.

- **Named-export invocation.** `gang run <robot> <cap> --export <name>`
  invokes a declared export instead of the default `run`; the runtime
  resolves exports by name, the wire field is additive (old agents ignore
  it), and manifests may declare their export set for pre-flight visibility
  (`gang sign --exports`). The enabler for multi-operation capability
  contracts such as adapter lifecycles. Closes #42.

- **Credential slots.** Manifests declare slot names (`gang sign
  --credential-slots`); robots bind slots to secret files in
  `~/.gang/credentials.toml`; the agent re-reads the file at every invoke
  (rotation without redeploy) and injects `GANG_CREDENTIAL_<SLOT>` into the
  sandbox's otherwise-empty WASI environment. Values never enter manifests,
  policy, logs, or the event feed; unbound slots are skipped loudly.
  `gang policy check` lists slots and exports. Closes #43.

## [2.4.0] - 2026-08-15

### Added

- **Policy re-sync sweep: timed grants now revoke, not just expire.** The
  robot agent reloads `policy.toml` every 60s (configurable) and revokes any
  installed capability no longer permitted — an expired `--until` grant or a
  rule narrowed on disk. Prevent-new semantics: in-flight invocations finish
  (the installed-map write lock waits on them), new ones are refused; the
  on-disk bundle moves to `.revoked/` so a restart cannot resurrect it; the
  revocation is emitted on the event feed, denial log (with remedy), and
  audit log exactly like a deploy-time refusal. Keep-last-good on runtime
  policy read errors (startup stays fail-closed per SEC-01). Side effect:
  `gang policy allow` applies within one sweep — no agent restart. Closes #37.

- **SYN-retransmit loss detection in `gang doctor --profile-out`.** A connect
  landing ≥800ms over the median almost certainly lost its first SYN to the
  kernel's 1s retransmission timer — that is how light (1–5%) loss presents
  in a connect probe without ever failing one. Loss events are now hard
  failures + retransmit detections; default samples raised 20→40 (~2.5%
  resolution); the profile header itemizes failures vs detections and the
  delay median stays robust to retransmit-inflated outliers. Closes #38.

- **Perf tripwires in the test suite.** `policy::evaluate_at` measured at
  ~624ns/op and the profile synthesis pipeline at ~4.4µs/op (release, 10k
  iters); tests assert generous ceilings (50µs / 100µs) so an
  order-of-magnitude regression fails CI without flaking on slow runners.

### Added

- **`gang doctor --profile-out` — the customer link as a CI test case.**
  Measures the actual link (median TCP connect RTT + failure rate against the
  configured relay) and emits a deterministic degraded-link fixture: fixed
  netem delay (RTT split evenly per side), statistic-nth loss, measured
  spread recorded in comments but deliberately not reproduced (gate
  determinism contract). Rates are operator-supplied (`--uplink-kbit`/
  `--downlink-kbit`) since a handshake probe cannot measure them — the header
  says which numbers were measured vs supplied. `run-matrix.sh
  --profile-file` replays any external fixture. Closes #33.

- **Time-boxed policy allows (`gang policy allow --until`).** The
  sudo-timestamp analog: `--until 2h` (or RFC3339) records the widening as a
  `timed_patterns` entry the engine ignores after expiry — enforcement at
  evaluation time, no daemon. Malformed expiries fail closed. Re-allowing
  without `--until` upgrades to permanent and drops the shadowed timed entry;
  `gang policy lint` flags expired leftovers as dead weight. Closes #34.

- **`gang policy check` — pre-flight a signed component against local
  policy.** The same evaluation and remedies as the deploy denial path, run
  offline before any robot is involved; every declared capability gets its
  own verdict, `--as-peer` evaluates someone else's deploy, non-zero exit on
  denial for capability-authoring CI. Closes #35.

- **Policy change audit (`gang policy history`).** Denials were already
  logged; now the widenings that answer them are too: every `gang policy
  allow`/`allow-peer` records who (local gang id), what (group, pattern,
  access, expiry), when, and why (`--reason`) to a size-capped
  `policy-history.jsonl`. Hand-edits bypass the trail and the rendering says
  so. Closes #36.

## [2.3.0] - 2026-08-14

### Added

- **Distribution channels.** `brew install robotden/tap/gang`
  (RobotDen/homebrew-tap, prebuilt binaries + shell completions),
  `cargo binstall gang` (release-tarball metadata), and Debian/Ubuntu
  packages (`cargo-deb`; built for amd64 + arm64 in the release workflow).

- **Actionable policy denials (`gang policy`).** Default-deny only survives
  contact with real operations when the narrow edit is easier than the
  wide-open one. Every policy denial now carries its own remedy: the deploy
  error names exactly what was refused, why (no rule / pattern not covered /
  access exceeds max / peer unauthorized), and the smallest policy change
  that would permit exactly that request — as both a ready-to-run
  `gang policy allow` command and a `policy.toml` snippet. The robot appends
  each denial to a size-capped `denials.jsonl` beside the audit log;
  `gang policy denials` reviews them firewall-log style (aggregated, newest
  first, remedy attached). `gang policy allow` applies the minimal widening
  with `visudo` semantics — mutate, re-validate, write atomically, keep a
  `.bak` — and refuses `**` without `--wide-open`. `gang policy lint`
  [`--strict`] is the drift tripwire for CI/cron: it flags wide-open
  patterns, read-write-on-everything, wildcard deploy rights, and
  missing-policy permissive mode.

- **Degraded-link CI matrix (`test-harness/degraded-link/`).** The
  e2e-dispatch round-trip (deploy → invoke → verify over the relay circuit)
  now runs as a required gate on every main push under five deterministic
  link profiles — clean, lossy (statistic-nth 3% loss + 40ms), high-latency
  (250ms RTT), asymmetric (192kbit uplink cap), and nat-relay (direct path
  firewalled; relay-only) — with a per-run JSON artifact recording the exact
  shaping commands. A nightly non-blocking chaos run applies seeded random
  netem impairment and opens an issue carrying the replay seed on failure.
  Gate profiles use only mechanisms that reproduce exactly run-to-run;
  netem's random distributions are chaos-only because its per-packet draw is
  kernel RNG and cannot be seeded. Closes #32.

- **Live ROS topic streaming + full Foxglove projection (`gang view
  --topics`).** A dedicated `/ganglion/topics/1.0` substream (mirroring the
  ADR-024 push shape): the operator sends one `TopicStreamRequest`; the robot
  authenticates the subscriber with the same trust rule as deploy, evaluates
  **each topic against the default-deny policy engine** as a read-only
  `ganglion:ros/interface` pattern (the `policy.toml` globs and read-only
  ceiling govern live streaming exactly as they govern deployed capabilities),
  emits a `PolicyDecision` per verdict on the event feed, replies with the
  per-topic verdicts, and streams samples live. Samples come from
  `ros2 topic echo` (RMW-agnostic, so Zenoh fleets work unchanged), are
  converted YAML→JSON on the robot (subset converter, unit-tested against
  ros2-echo document shapes), and are **shaped robot-side** (decimation,
  per-message size cap, rate ceiling from the `--profile` knobs) before
  crossing the wire. `gang view <robot> --topics /a,/b` advertises one
  Foxglove channel per permitted topic alongside `/ganglion/events`.
  Closes #27; completes #18.

- **`scripts/seed-registry.sh` — one-command registry seeding.** Builds all
  eight in-tree capability crates as WASM components (cargo-component,
  wasm32-wasip2), signs each with your identity key declaring exactly the
  capability groups it needs, and publishes them to the open registry.
  `--dry-run` prints the crate→capabilities mapping without building. (#21)

### Fixed

- **crates.io publish surface.** Every published crate now packages the
  repository README (the crate pages rendered "appears to have no README.md
  file"), carries keywords + categories for crates.io browsing, and
  gang-core's module index no longer renders doubled summaries on docs.rs.

## [2.2.0] - 2026-08-10

### Added

- **`gang doctor` — print exactly what the network permits.** A field-facing
  egress diagnostic: probes outbound TCP 443, UDP/QUIC, non-443 TCP, DNS, and
  (when configured or passed with `--relay`) reachability of the relay's
  transport address, then prints a PASS/FAIL table plus a copy-pasteable
  egress allowlist for the customer's network/security team. `--format json`
  for machines; exits non-zero when no viable outbound path exists so it works
  as a script/CI gate. Closes #17.

- **Bandwidth profiles (`gang profiles`).** Named degraded-link presets —
  `full`, `lidar-low`, `vision-low`, `logs-only` — as a shared
  `gang_core::bandwidth` type: decimation, per-message size cap, and a rate
  ceiling. Operator-defined profiles merge in from `~/.gang/config.toml`.
  Accepted via `--profile` on streaming surfaces. Closes #22.

- **`gang view` — bridge a robot's live feed into Foxglove/Lichtblick.** A
  dependency-light Foxglove WebSocket server (handshake, framing,
  serverInfo/advertise, MessageData — protocol layer unit-tested against the
  RFC 6455 and SHA-1 test vectors) plus `gang view <robot>`, which opens
  `ws://127.0.0.1:<port>` and forwards the relay-delivered, capability-scoped
  event feed as the `/ganglion/events` JSON channel with `--profile` shaping.
  Live ROS topic projection rides the same bridge and is tracked in #27;
  `--topics` is reserved for it. (#18)

- **`gang mcp` — serve Ganglion tools to AI agents over MCP (stdio).** A
  Model Context Protocol server exposing a curated, read-only fleet-discovery
  toolset (`gang_status`, `list_peers`, `list_capabilities`, `network_doctor`,
  `list_bandwidth_profiles`) over line-delimited JSON-RPC 2.0. The sandbox,
  signed manifests, default-deny policy, and audit log mean an agent provably
  cannot exceed what those mechanisms permit. Mutating tools are deliberately
  deferred and will be policy-checked and audited like the CLI. Closes #20.

- **`gang alert` — the metric→threshold→webhook primitive.** Rules (metric,
  comparator, threshold, cooldown) in operator config fire a
  Slack-incoming-webhook-compatible JSON payload on breach; `gang alert check`
  evaluates, `gang alert test` fires a sample, `--dry-run` prints instead of
  POSTing. Deliberately the useful 20% — incident-management integrations stay
  out of the open core. Closes #23.

- **`gang status --html` — a self-contained fleet-status snapshot.** Renders
  identity, registered peers, capability count, and recent audit from local
  state into a single shareable HTML file (escaped, unit-tested renderer).
  Not a live dashboard — that's `gang tui`. Closes #26.

- **`gang new tool` — the guided capability author loop.** Scaffolds a
  capability project and prints the full idea → build → sign → publish path to
  the open registry. (#21)

- **Integration docs:** `docs/ZENOH.md` (Ganglion composes with `rmw_zenoh` —
  the brokers drive the RMW-agnostic `ros2` CLI; Zenoh moves data, Ganglion
  adds outbound reach, identity, capability-scoped tooling, and audit — closes
  #24) and `docs/OTA.md` (pair with Mender/balena rather than building OTA —
  closes #25).

- **`gang tui` — the live fleet dashboard (issue #2).** A full-screen
  [ratatui](https://ratatui.rs) dashboard that subscribes to every registered
  robot's event feed (the ADR-022 layer) and folds it into four live panes:
  **Peers** (status dot `●`live/`◐`transitional/`○`offline · transport · RTT),
  **Tunnels** (direct vs relay · ↑/↓ byte counters from the live transport
  stats), **Policy decisions** (ts · ALLOW/DENY · capability group · operator ·
  reason), and a tailing **Audit** log (ts · action · result · duration). A
  title bar shows relay, live/total peers, uptime, and a `♥ live` pulse that
  becomes `[stale feed]` when the feed goes quiet. Keys: `↑↓`/`j k` select a
  peer, `⏎` inspect it (a drill-down overlay of caps + recent decisions +
  audit), `p` pause/resume the feed (freezes the display with a `PAUSED`
  indicator and replays buffered events on resume — for clean demo captures),
  `/` filter, `a` audit-only fullscreen, `?` help, `q`/Esc quit. With no robots
  registered it shows a friendly first-run panel pointing at `gang up` /
  `gang pair`. Honors `NO_COLOR` (monochrome/ASCII theme — ASCII borders and
  status markers, no color escapes) and is resize-aware (collapses to a single
  stacked column on narrow terminals, a "terminal too small" hint below the
  minimum). The core is structured for headless testing: a **pure event-fold
  state reducer** (synthetic `AgentEvent`s → asserted UI rows) and
  widget-render tests over `ratatui::backend::TestBackend`, with `main`/the
  event loop a thin shell; subscriptions run on tokio tasks feeding the render
  loop over a channel so the UI never blocks on the network, and the terminal
  is restored on quit/Ctrl-C/panic. Feed delivery is the ADR-024 push substream
  (events land instantly; heartbeats still drive staleness). A headless
  `--frames N` snapshot mode renders a
  frame to text for CI and capture; `--robot <name>` focuses one robot. Design
  notes in [ADR-023](docs/adr/ADR-023-tui-dashboard.md). Adds `ratatui` 0.30 and
  `crossterm` 0.29 to the workspace (used in `gang-cli` only; kept out of the
  library crates).

- **Presence & event-streaming layer — `gang logs`/`connect`/`transport-stats`/`list`
  are now real.** An authenticated, bounded robot→operator event feed carries
  presence, policy decisions (allow and deny), audit appends, connection changes,
  and heartbeats. A versioned, `#[non_exhaustive]` `AgentEvent` enum lives in
  `gang_core::events`, framed with the existing CBOR length-prefixed codec (no
  second codec). The robot side (`gang_ros::events`) is a bounded event bus: a
  `tokio::sync::broadcast` channel (capacity 256) for live consumers where a
  lagging subscriber degrades to a `Gap{dropped}` marker instead of growing
  memory, plus a 256-entry recent-events ring for polling/late subscribers.
  Emission is wired at the real sites — `PolicyDecision` at every deploy
  `policy.evaluate` and at sandbox policy-denied invokes, `AuditAppended` on
  every audit append, a `PresenceSnapshot` per subscription, a 15 s `Heartbeat`,
  and `ConnectionChanged` from transport events. Subscribing is authenticated by
  the **same trust rule as deploy** (only trusted operators when a trust store is
  configured; loud dev-permissive when empty); an unauthorized peer is never
  streamed to, and events carry **no secret material**. `gang logs <robot>
  [--follow] [--since <dur>]` prints audit + policy events as human lines or
  JSONL; `gang connect <robot>` is a live scrolling status view (the non-TUI
  precursor to the dashboard); `gang transport-stats <robot>` returns **real**
  live-circuit counters (the simulated data is gone); `gang list` shows
  registered robots with live reachability from a presence probe. The operator
  API is `Libp2pTransportAdapter::subscribe_events`. A distinct
  `/ganglion/events/1.0` protocol id is reserved (and served) for a future push
  substream; today the feed rides the control protocol as a bounded poll because
  libp2p 0.56's request-response always negotiates the first protocol and
  `libp2p-stream` is not in the workspace dependency table. Full design and
  trust/resource model in
  [ADR-022](docs/adr/ADR-022-event-subscription-layer.md); coverage is unit tests
  for the bounded bus (lag→`Gap`, ring eviction→`Gap`, subscriber auth) plus an
  in-process integration test over a real relay circuit (authorized subscribe →
  snapshot; deny → `PolicyDecision{Deny}`; invoke → `AuditAppended`; unauthorized
  refused). No new workspace dependencies (`futures` — already in the table — is
  added to `gang-ros`).

- **`gang pair` / `gang join` — one-line robot enrollment.** The Tailscale move
  for robots: the operator runs `gang pair` and gets ONE copy-paste line; the
  robot runs `gang join gang1_…`, dials out, and appears in `gang peer list`
  ready for `gang deploy`/`run`/`caps` — no manual id copying in either
  direction. `gang pair` mints a short-lived, single-use **pairing token** bound
  to the relay and the operator's identity (versioned, URL-safe, self-describing
  with expiry), reserves a relay circuit, and waits; `gang join` decodes the
  token, loads/generates the robot identity, trusts the operator, dials the
  operator through the circuit, enrolls, and then stays online as the agent
  (`--once` to enroll and exit). The operator records the robot under the
  identity **libp2p authenticates on the wire** — never a self-report — and
  accepts a self-reported dialable id only after confirming it embeds that same
  key; tokens are single-use and expiring, and forged/tampered/expired/reused
  tokens are rejected. Full trust model in
  [ADR-021](docs/adr/ADR-021-pairing-token-enrollment.md); token mint/verify
  logic (with base64url + CBOR encoding, no new dependencies) lives in
  `gang_core::pairing` with unit tests, and loopback integration tests cover the
  happy path plus reuse/expiry/wrong-identity rejections. Flags: `gang pair`
  `--relay`, `--name`, `--expires`, `--qr`, `--timeout`, `--json`; `gang join`
  `--name`, `--once`, `--timeout`, `--json`; both honor the global `--data-dir`.
  QR output is a documented follow-up (no terminal-QR crate is in the workspace
  dependency table; `--qr` prints the copy-paste line rather than adding an
  unapproved dependency). The manual `gang peer add` remains as the fallback.
- **`gang init` — guided first-run setup.** Takes a fresh install from
  *installed* to *configured* in one command: runs the same archetype probes as
  `gang diagnose` and prints the detected network archetype plus its transport
  implication; generates the operator identity if none exists; writes a
  genuinely **default-deny** `policy.toml` (no capability group permitted, with
  clearly-commented example rules to uncomment) and an operator `config.toml`
  (defaults incl. `host_key_policy = strict`); and prints a short,
  correctly-ordered next-steps panel tailored to the archetype (every printed
  command is real and runnable). Interactive on a TTY with skippable `[Y/n]`
  prompts, and degrades cleanly to non-interactive on a pipe/CI or with `--yes`.
  Idempotent: re-running reports and keeps existing files, never clobbering an
  identity, policy, or config without `--force`. Flags: `--data-dir`, `--force`,
  `-y`/`--yes` (alias `--non-interactive`), `--json`.
- **`gang up` — one command to a real local fleet.** Bridges the gap between
  `gang demo` (self-contained, tears itself down) and a hand-wired
  relay/agent/deploy. In one foreground command it stands up a loopback circuit
  relay, a robot agent with a real **default-deny** policy on disk (only the
  sample's diagnostics group is permitted; commented examples show how to widen
  it), and one sample capability signed with the operator identity, then
  registers the robot as `up-robot` and prints the exact `gang` commands to
  drive it from another terminal. Relay and agent run as in-process tasks in a
  single runtime (mirroring the e2e harness) so Ctrl-C tears the whole fleet
  down cleanly. Flags: `--data-dir`, `--port`, `--force`, `--json`. Alias:
  `gang fleet`.
- **Global `--data-dir` flag.** Points the whole CLI at a self-contained fleet
  directory instead of `~/.gang` (identity, peer registry, config, trust store),
  via the `GANG_HOME` environment variable. `gang --data-dir <dir> deploy
  up-robot …` drives a `gang up` fleet.

### Changed

- **README repositioned.** Leads with what Ganglion is and where it fits
  (differentiation table, "already on Tailscale + SSH? keep them"), trims the
  quickstart to three commands, links the full walkthroughs, makes the
  **Apache-2.0-forever** commitment explicit, and points FleetLink references
  at the dedicated landing pages (tafylabs.io/fleetlink, with the
  reviewer-facing security overview at tafylabs.io/fleetlink/security linked
  from the security docs). Demo GIF replaced with a real `gang up` → `gang tui`
  capture rendered through the actual TUI renderer.

- **Shell scripts are linted.** The pre-commit hook shellchecks
  `scripts/*.sh` and both git hooks (skipped when shellcheck is absent), and
  CI gained a Shellcheck job.

- **Dependency policy documented.** `libp2p 0.56 → libp2p-identity 0.2 →
  ed25519-dalek 2` is one coupled version train (noted in `Cargo.toml`);
  dependabot majors that split it (#15, #16) are declined until a libp2p
  release moves the whole train. Grouped minor/patch bumps merged (#14).

- **Event feed defaults to genuine server-push, with a selectable poll fallback
  (ADR-024).** The robot→operator event feed (`gang logs --follow`,
  `gang connect`, `gang tui`, and the `list`/presence probe) gains a persistent
  push substream on `/ganglion/events/1.0`, carried by the new `libp2p-stream`
  dependency. The robot accepts inbound event substreams, authenticates each
  subscriber with the **same** trust rule as deploy (SEC-03; loud dev-permissive
  on an empty trust store), sends the `PresenceSnapshot` + retained catch-up,
  then pushes framed `AgentEvent`s live from the bounded `EventBus` broadcast
  until the stream closes (slow consumer → `Gap`, unchanged). The operator's
  `Libp2pTransportAdapter::subscribe_events` returns a live `Stream<AgentEvent>`
  (`EventFeed`) carried by either transport. Events reach operators the instant
  the robot emits them — measured ~2 ms over a real relay circuit (asserted
  `< 500 ms`), versus the ~1.5 s poll cadence.

  Because `libp2p-stream` is a pre-release, the request-response poll from
  ADR-022 (`ControlMessage::SubscribeEvents`) is **retained as a fallback**, not
  removed. A new `events_transport` operator-config field and a per-command
  `--events-transport <auto|push|poll>` flag on `logs`/`connect`/`tui` select
  the transport: `auto` (default) prefers push and falls back to poll
  automatically when push is unavailable (older/alpha-free agent,
  protocol-not-supported, alpha misbehaving) or if a push stream drops
  mid-session; `push` forces the stream (clear error if unavailable, no silent
  poll); `poll` forces the request-response loop (interval configurable via
  `events_poll_interval_ms`, default 1500 ms). `gang connect`/`tui` show a
  subtle `feed: push` / `feed: poll (1.5s)` indicator. The `AgentEvent` wire
  model, the trust rule, and the bounded resource model are unchanged. Adds
  exactly one workspace dependency, `libp2p-stream` (pinned `=0.4.0-alpha`;
  resolves cleanly against the locked `libp2p-swarm 0.47.1`); `cargo deny`
  passes clean, so no policy exception was needed. Streams traverse the relay
  circuit end-to-end, same as control RPC. See
  [ADR-024](docs/adr/ADR-024-event-push-stream.md).

### Fixed

- **`OperatorConfig::default()` now yields the documented defaults.** The
  `#[serde(default = "…")]` attribute only fires during deserialization, so a
  derived `Default` produced an empty `host_key_policy`, which `verify_host_key`
  then rejected in any environment without a `config.toml` (e.g. a fresh fleet
  directory). `Default` is now hand-written to match the deserialized defaults
  (`host_key_policy = "strict"`).


## [2.1.0] - 2026-08-06

### Fixed

- **WASI-built components can actually run.** The component runtime now links
  a locked-down WASI 0.2 host (no environment, no arguments, no preopens,
  sockets denied, stdin closed, stdout/stderr captured — bounded — for
  diagnostics). Components produced by standard toolchains (cargo-component
  wraps a wasm32-wasip1 core module with the WASI preview1 adapter) import
  `wasi:cli`/`wasi:io`/`wasi:clocks`/`wasi:filesystem`/`wasi:random` even when
  they never touch the system; without the WASI host definitions every such
  component failed to instantiate on its first `gang run`.
- **Record-typed host returns are really records.** Host functions whose WIT
  signature returns records (`system-info`, `network-state`, `list-ros`,
  `list-sources`, `stat-file`, `spawn`, `spawn-with-env`, `ping`,
  `dns-lookup`, `port-check`, `traceroute`) previously returned JSON bytes;
  Wasmtime type-checks dynamic host-function results against the component's
  expected type, so the first such call trapped with
  `type mismatch: expected record, found u8`. Broker JSON is now converted
  into properly-typed component values.

### Added

- **Operator-side remote dispatch (ADR-020 Phase 32).** `gang deploy`, `gang
  run`, and `gang caps` against a remote robot now work end-to-end: the CLI
  builds a libp2p transport from the operator identity, dials the configured
  relay, dials the robot via `<relay>/p2p-circuit/p2p/<robot-libp2p-id>`, and
  exchanges control messages on `/ganglion/control/1.0` (fresh nonce +
  timestamp per request; the robot rejects replays). Remote failures exit
  non-zero with actionable errors; the whole dispatch is timeout-bounded
  (60 s deploy, 30 s run/caps, `--timeout <secs>` to override).
- **Dialable peer ids in the registry.** `gang peer add` accepts either the
  base58 libp2p id (`12D3KooW…`, printed by `gang agent`/`gang relay` as
  `Peer ID (libp2p/dial)`) — deriving and storing the gang id alongside — or
  a legacy gang id (stored without a dial id; remote dispatch then instructs
  re-adding with the libp2p form). `peer list`/`show` display both. Older
  `peers.json` files load unchanged.
- **Host-key verification enforced.** The SSH-style TOFU machinery now gates
  every remote dispatch: `strict` prompts on first connect (and refuses
  non-interactive stdin with guidance), `tofu` auto-accepts the first key;
  both hard-fail with the loud SSH-style warning when a known robot name
  presents a different identity. `gang peer trust-reset` clears it.
- **Relay circuit reservations.** Client nodes with configured relays listen
  on `<relay>/p2p-circuit`, establishing — and automatically re-establishing —
  the reservation that makes them reachable through the relay. The relay
  server registers its listen addresses as external addresses (reservations
  were previously unusable: `NoAddressesInReservation`) and lifts the libp2p
  per-circuit limits (128 KiB / 2 min → unlimited bytes / 1 h) so deploys can
  ship component bytes.
- **In-process integration tests** (`crates/gang-cli/tests/remote_dispatch.rs`)
  drive Deploy → Invoke → List through a real relay circuit on loopback (the
  robot binds no direct address), plus replay rejection, untrusted-deployer
  rejection, and a direct-dial variant.
- The e2e Docker harness (`test-harness/e2e-dispatch/`) performs a real
  deploy → invoke → list round-trip with the signed test WASM component
  instead of a connectivity-only smoke test.

### Changed

- `gang agent` (relay mode) prints `Peer ID (libp2p/dial): …` and a
  copy-pasteable `gang peer add` line using the dialable id, and requires the
  relay multiaddr to carry its `/p2p/<relay-libp2p-id>` suffix (failing fast
  with guidance otherwise — reservations cannot be requested without it).
- Control-plane RPC timeout ceiling raised from libp2p's 10 s default to
  120 s; `send_rpc` supports per-request timeouts.
- `gang logs`/`list`/`connect`/`transport-stats` remain `[WIP]`, now
  explicitly waiting on the presence/streaming layer (fleet discovery and
  long-lived sessions), not on remote dispatch.

## [2.0.0] - 2026-07-23

A security- and quality-hardening release. Every change below reflects code that
has landed. See [docs/MIGRATION-v2.md](docs/MIGRATION-v2.md) for upgrade steps.

### ⚠️ Breaking changes

- **Unified peer-id derivation (SEC-03).** The libp2p transport now derives a
  remote peer's gang id from its raw Ed25519 public key (recovered from the
  libp2p peer id) using the *same* scheme as `gang-core`. Previously the
  transport used a libp2p-multihash-based id that never matched the core
  derivation, so trust-store `peer_rules` were not actually enforceable. They
  are now. A robot's own id (derived from its key) is unchanged, but any
  tooling/config that recorded the old multihash-based remote id must be
  regenerated.
- **Fail-closed policy and trust store.** A malformed or unreadable policy or
  trust-store file now aborts agent startup instead of silently falling back to
  a permissive policy. Deployments that relied on the old permissive fallback
  will now fail loudly until the file is fixed.
- **Replay protection on control requests.** Control requests now carry a
  per-request nonce and timestamp; the agent rejects stale or replayed
  requests. This is an additive wire change, but a pre-2.0 operator that sends
  requests without the nonce is rejected — **all agents and operators must be
  upgraded together.**
- **`registry publish` requires a signed manifest (SEC-15).** `Registry::publish`
  now takes a `&SignedManifest` and authenticates every entry against it;
  publishing without an adjacent signed manifest is no longer possible.
- **Library API changes.** `Cid::parse` (fallible parse replacing loose string
  handling), `Registry::publish(entry, &signed_manifest)`, and strict `PeerId`
  validation (`PeerId::parse`/`from_str` reject malformed ids) change public
  signatures. Public wire enums (`ControlMessage`, `InvokeStatus`,
  `BrokerOperation`) are now `#[non_exhaustive]`, so downstream `match`
  statements must add a wildcard arm.
- **Reduced library tokio feature set (CODE-15).** Library crates
  (`gang-core`, `gang-libp2p`, `gang-ros`) now depend on a minimal tokio
  feature set; the `gang` binary widens it to `full`. Downstream consumers of
  the library crates that relied on transitively-enabled tokio features must
  enable them explicitly.

### Security

- **SEC-03** — Peer-id derivation unified across the libp2p transport and
  `gang-core`, making trust-store `peer_rules` enforceable (see breaking notes).
- **Fail-closed policy/trust loading** — malformed policy or trust store aborts
  startup; no permissive fallback.
- **Identity key permissions enforced** — identity key files with permissions
  looser than `0600` are repaired to `0600` (with a warning) before use.
- **Hardened WASM execution path** — the runtime enforces manifest-derived
  memory and fuel limits, re-hashes component bytes and refuses to execute on a
  Blake3 mismatch, and has no silent WASM→broker fallback (a WASM failure is
  terminal).
- **Tamper-evident audit log** — the audit log is now a Blake3 hash chain
  (`blake3(prev_hash || seq || cbor(record))`) with a `verify_chain()`
  integrity check and `0600` permissions; reordering or deletion is detectable.
- **Replay protection (control plane)** — nonce + timestamp on control
  requests; stale/replayed requests are rejected.
- **Network-probe broker SSRF hardening** — blocks loopback, link-local, the
  cloud-metadata address (169.254.169.254), and IPv6 ULA ranges
  unconditionally, and enforces a host/CIDR allowlist.
- **Process broker hardening** — requires absolute, allowlisted command paths
  (matched after canonicalization) and scrubs the environment (`env_clear`)
  before spawning.
- **Filesystem broker TOCTOU closure (SEC-10)** — operates on canonicalized
  paths, including canonicalizing the parent for writes to new files.
- **SEC-15** — registry entries authenticated against signed manifests (see
  breaking notes). The registry additionally validates each entry
  **field-by-field** against the signed manifest — name, version, capabilities,
  and component CID must all match; `gang registry publish` rejects a
  `--version` override that contradicts the manifest.
- **Network-probe DNS-rebinding resistance** — probes resolve the target once,
  vet the canonicalized addresses, and then connect only to those vetted
  addresses (the hostname is never re-resolved for the connection). Blocked
  ranges now also include IPv4-mapped-IPv6 addresses and IPv6 link-local
  (`fe80::/10`).
- **Filesystem broker final-component symlink rejection** — writes to new files
  reject a symlink (including a dangling symlink) at the final path component,
  closing the remaining symlink-planting window on the new-file write path.
- **Replay guard capacity bound** — the control-plane replay guard has a hard
  capacity of 100,000 tracked nonces and fails closed (rejects requests) when
  full, bounding memory while never degrading to accepting replays.
- **Audit chain rotation linkage** — on size-based rotation, the new audit log
  file carries the rotated file's tip hash, so the hash chain spans rotations.
  The audit documentation now states the honest trust bounds: without an
  external anchor, a full rewrite of the log is undetectable, and trailing
  truncation is undetectable.
- **Dependency advisories cleared** — wasmtime upgraded 29 → 36 (LTS),
  clearing six RUSTSEC advisories against wasmtime/wasmtime-wasi 29.x
  (RUSTSEC-2025-0046, 2025-0118, 2026-0006, 2026-0020, 2026-0021, 2026-0085);
  the two hickory-proto advisories (RUSTSEC-2026-0118/0119, DoS-class,
  transitive via libp2p-dns) are documented-ignored in `deny.toml` pending a
  libp2p bump to hickory 0.26; the cargo-deny unmaintained policy is now
  scoped to direct workspace dependencies (`unmaintained = "workspace"`).

### Changed

- `gang sign` now takes an explicit `--capabilities` flag instead of
  auto-extracting capabilities from the component; when omitted it falls back to
  a permissive default set and prints a loud warning. `--component-version`
  (alias `--version`) sets the component's semantic version, distinct from the
  CLI's own `-V`.
- `gang registry publish` requires a signed manifest and gains `--version` and
  `--language` overrides.
- `gang capability scaffold` now writes a real `wit/ganglion.wit` into the
  generated project (embedded from the canonical in-repo WIT), rather than
  telling the author to copy it by hand.
- `gang logs`, `gang list`, `gang connect` are marked `[WIP]` and now exit
  non-zero; `gang transport-stats` output is explicitly labeled simulated.
- `RUST_LOG` is now honored; added a `-q`/`--quiet` global flag.
- `gang relay` gains `--data-dir` for the persisted identity key; the key path
  is plumbed directly to the relay (no environment variable is set or read at
  relay runtime).
- `gang demo` keeps its data under `/tmp/gang-demo` and prints a cleanup hint.
- `gang agent -r <relay>` no longer hangs or exits when the relay is
  unreachable: it warns, keeps serving, and retries the relay dial every 5
  seconds.
- `gang peer add` rejects a malformed peer id with a clean error instead of
  panicking.
- CLI polish: `gang --help` prints a long description and an after-help pointer
  to `gang demo`; subcommand aliases `id` (identity), `cap` (capability), and
  `dx` (diagnose); `gang status` shows the registry
  (`~/.local/share/gang/registry`) and artifact-store data directories.
- `gang test-archetype` polls container state until services are up instead of
  sleeping for a fixed interval.
- The WASM runtime caches compiled components (bounded at 64 entries; evicted
  components are recompiled on next use), and the transport's event fan-out
  prunes closed subscribers with in-flight outbound requests capped at 1024.
- **MSRV raised to Rust 1.88** — declared as the workspace `rust-version`,
  checked by a dedicated CI job, and used by both Dockerfiles
  (`rust:1.88-slim-bookworm`).
- **CI expanded** — jobs now cover fmt, clippy, test (Ubuntu + macOS), rustdoc
  (`-Dwarnings`), MSRV (1.88), dependency audit (`cargo-deny`), and the Docker
  test harness on pushes to `main` (blocking: open-warehouse + e2e-dispatch
  smoke; non-blocking: nat-office, enterprise-dmz, mobile-cgnat).

### Fixed / clarified WIP

- **Operator remote dispatch is still WIP.** `gang deploy`/`run`/`caps` against a
  remote robot resolve the target then exit with a clear "not yet implemented
  (ADR-020 Phase 32)" message; the local fallback path works. Earlier changelog
  entries that implied relay-mediated remote dispatch had shipped were
  incorrect — see the corrected v0.6.0 entry below.
- The `e2e-dispatch` harness is an honest connectivity smoke test, not a full
  remote deploy/invoke round-trip.

### Docs

- Documentation truth pass: corrected test counts (now **337** across 13
  crates, plus 1 ignored live-network test), the crate dependency graph, the standard-library table (adds
  rosbag-slice), the canonical repository URL (`RobotDen/ganglion`), and CLI
  reference transcripts/flags to match real output.
- Added [docs/MIGRATION-v2.md](docs/MIGRATION-v2.md) and a
  [docs/README.md](docs/README.md) documentation index.
- Marked the four-repos layout and stale dependency table in IMPLEMENTATION.md,
  and DesignSpec.md, as historical.
- `LICENSE` and `NOTICE` (Apache-2.0) now exist at the repository root.

### Tests

- **337 total tests passing** across 13 crates (82 added since v1.0.0), plus
  **1 ignored** live-network archetype test (run with `cargo test -- --ignored`).

## [1.0.0] — 2026-04-24

### Stability commitments

Ganglion v1.0 marks the first stability commitment. The following surfaces are now stable:

- **Stream protocols**: `/ganglion/control/1.0`, `/ganglion/tool/1.0`, `/ganglion/bulk/1.0` — wire format and framing are frozen. Future versions will negotiate via protocol ID versioning.
- **WIT interfaces**: `ganglion:capability@0.5.0` — all eight capability group interfaces are stable. New interfaces may be added in future minor versions; existing interfaces will not break within a major version.
- **CLI surface**: All commands documented in `docs/CLI_REFERENCE.md` are stable. Commands marked `[WIP]` (`gang list`, `gang connect`) may change. New commands may be added.
- **Manifest schema**: v2.0 is stable. Future fields will use `#[serde(default)]` for backward compatibility.

### Added

- **libp2p 0.56 upgrade**: Updated from libp2p 0.54 to 0.56. Removes async-std (tokio-only), adds peer-store support. One code change for request-response API evolution.
- **Happy-eyeballs `dial_parallel()`** (`gang-core`): Real implementation replacing the stub — stagger delays between transport attempts, first successful connection wins, timeout enforcement, and transport filtering against capabilities. 7 tests with MockTransport.
- **WebTransport/WebRTC preparation** (`gang-libp2p`): Config flags (`enable_webtransport`, `enable_webrtc`), capability reporting, and multiaddr transport detection. Native transport integration deferred — no libp2p release (including 0.56) provides native WebTransport or WebRTC. Config and detection are in place for when upstream ships native support.
- **Decision flowchart** (`docs/decision-flowchart.svg`): One-page architectural selection flowchart mapping network archetype → transport strategy → relay requirements.
- **Rosbag slicing capability** (`gang-capability-rosbag-slice`): Time-bounded rosbag2 slice configuration with relative time parsing, topic filtering, sqlite3/mcap format support. 19 tests.
- **Multi-language reference implementations** (`examples/`): Python log-normalize (componentize-py), C++ topic-echo (wasi-sdk + wit-bindgen), Go canary-probe (TinyGo).
- **Standard library completion**: log-normalize (11 tests), topic-echo (11 tests), canary-probe (11 tests).
- **Capability Author Guide** (`docs/CAPABILITY_AUTHOR_GUIDE.md`): Comprehensive guide for Rust, C++, Python, and Go.
- **Performance targets** in `docs/VALIDATION.md`: Expected RTT, throughput, and connection times per network archetype.
- **255 total tests passing** across 13 crates (measured from the v1.0.0 tag).

### Known limitations

- Native WebTransport blocked by upstream (libp2p draft PR #4348, tracking issue #2993)
- Native WebRTC blocked by upstream (libp2p-webrtc v0.9.0-alpha.1 exists but is alpha, Linux-only)
- Docker e2e measured metrics pending Docker Desktop restoration
- `gang list` and `gang connect` remain WIP stubs requiring relay connectivity

---

## [0.6.0] — 2026-04-24

### Added

- **Robot agent serve loop** (`gang-ros`): `RobotAgent::serve()` registers a handler on `/ganglion/control/1.0` that deserializes incoming `ControlMessage` requests and dispatches to `deploy_capability()`, `invoke_capability()`, and `list_capabilities()` (ADR-020 Phase 32).
- **Agent transport startup** (`gang-cli`): `gang agent -r <relay>` creates a libp2p transport, dials the relay, registers the control handler, and runs the event loop. Without `-r`, agent runs in local mode for backward compatibility (ADR-020 Phase 33).
- **Peer registry CLI** (`gang-cli`): `gang peer add/remove/list/show/rename/trust-reset` subcommands for managing known peers stored in `~/.gang/peers.json` (ADR-020 Phase 34).
- **Operator target resolution** (`gang-cli`): Unified target resolution chain for `gang deploy`, `gang run`, and `gang caps`: registered name → abbreviated peer ID prefix (Docker-style) → full peer ID → local fallback. `-p`/`--peer` and `-r`/`--relay` flags on all commands (ADR-020 Phase 35). NOTE: relay-mediated *remote* dispatch is not yet wired — a resolved remote target exits with a "not yet implemented (ADR-020 Phase 32)" message; only the local fallback executes.
- **SSH-style identity verification** (`gang-cli`): `verify_host_key()` with three policies: `strict` (TOFU with interactive prompt, hard fail on mismatch), `tofu` (auto-accept, hard fail on mismatch), `none` (development only). SSH-style warning banner on key change (ADR-020 Phase 36).
- **Operator config file** (`gang-cli`): `~/.gang/config.toml` with `default_relay` and `host_key_policy`. `gang config show/set/init/path` subcommands. Config integrates into target resolution relay fallback chain (ADR-020 Phase 37).
- **Shell completions** (`gang-cli`): `gang completions <shell>` for bash, zsh, fish, elvish, and powershell via `clap_complete` (ADR-020 Phase 38).
- **`send_rpc()` method** (`gang-libp2p`): Send request bytes to a connected peer and await response bytes, used for operator→robot control messages.
- **`TrustStore::index_of()`** (`gang-core`): Locate the index of a trusted peer entry for SSH-style "offending key at index N" messages.
- **`PeerId::new()`, `PeerId::starts_with()`** (`gang-core`): Construct peer IDs from strings and check prefixes for Docker-style abbreviated matching.
- **`PeerRegistry::lookup_by_prefix()`** (`gang-core`): Find peers by abbreviated peer ID prefix.
- **`gang status` enhancements** (`gang-cli`): Now shows peer count, config path, default relay, and lists all new commands.

> The standard library completion, Capability Author Guide, decision flowchart,
> happy-eyeballs `dial_parallel()`, WebTransport/WebRTC preparation, rosbag
> slicing capability, and multi-language reference implementations were
> previously double-listed here; they are recorded once under **[1.0.0]** above.

### Changed

- `gang deploy`, `gang run`, `gang caps` accept `-p`/`--peer` and `-r`/`--relay` flags.
- `gang agent` accepts `-r`/`--relay` flag for remote mode.
- **189 total tests passing** across 13 crates at the v0.6.0 milestone. (An
  earlier version of this entry claimed 221; 189 is the count measured from the
  v0.6.0 tag.)

## [0.5.0] — 2026-04-23

### Security

- **Filesystem broker symlink jail bypass** (`gang-ros`): Write operations to new files now canonicalize the parent directory, preventing path traversal via `../` or symlinked parents (ADR-015).
- **RosList access control enforcement** (`gang-ros`): `RosList` now filters results through allowed patterns — components can only see topics/services/nodes matching their policy.
- **Rosbridge naming correction** (`gang-ros`): Renamed `rosbridge_available` → `ros2_available` and `check_rosbridge()` → `check_ros2_available()` to accurately describe the current implementation (ros2 CLI, not WebSocket rosbridge).

### Added

- **WASM-to-broker glue layer** (`gang-wasm-host`): New `imports` module registers async host functions for all 8 WIT capability interfaces on the Wasmtime linker. WASM components can now call broker operations through their declared WIT imports, completing the Layer 2 → Layer 3 bridge that was the project's central architectural gap. Includes Val extraction helpers and JSON serialization across the WASM boundary.
- **WASM execution path in robot agent** (`gang-ros`): `invoke_capability()` now attempts WASM execution when component bytes contain valid WASM (`\0asm` magic header). Falls back to direct broker invocation for non-WASM capabilities.
- **Reference diagnostic capability** (`gang-capability-diagnostics`): Full implementation with `DiagnosticReport` struct, `collect()` function, `format_report()` output, and 6 tests. Replaces the empty 2-line stub.
- **ROS broker operations** (`gang-ros`): `ServiceCall`, `ParamGet`, and `ParamSet` broker operations with structured rosbridge-protocol responses (ADR-016, ADR-017).
- **`param-set` WIT operation** (`gang-wasm-host`): Added to `ros-interface`; WIT package version bumped to `@0.5.0` (ADR-017).
- **`gang status` CLI command** (`gang-cli`): Reports version, identity, registry, available and WIP capabilities (ADR-018).
- **Real CID in `gang deploy`** (`gang-cli`): Deploy now computes actual CID from manifest bytes instead of using a hardcoded placeholder.
- **Capability loading on agent startup** (`gang-ros`): `load_installed_capabilities()` now scans the capabilities directory, deserializes manifests, verifies signatures and trust store, and logs warnings for failures.
- **49 new tests** across 5 crates. **175 total tests passing.** (An earlier
  version of this entry claimed 188; 175 is the count measured from the v0.5.0
  tag.)

### Changed

- `BrokerOperation` enum gains `ParamSet` variant.
- WIT package version `ganglion:capability@0.5.0`.
- Wasmtime engine configured with `async_support(true)` for correct async instantiation.
- `gang-ros` depends on `gang-wasm-host` for WASM execution path.

---

## [0.4.0] — 2026-04-23

### Added

- **Expanded capability interface** (`gang-core`, `gang-ros`): Three new WIT capability groups — `ganglion:process/spawn@1.0` (bounded subprocess invocation with command allowlist), `ganglion:network/probe@1.0` (ping, DNS, port check, traceroute), `ganglion:metrics/emit@1.0` (structured metric emission with ring buffer). Full Layer 3 broker implementations for all three.
- **Standard capability library**: Three Rust capability crates — `gang-capability-param-inspect` (parameter server snapshot with diff), `gang-capability-diagnostic-bundle` (v2 comprehensive diagnostics with automated health checks), `gang-capability-network-archetype` (v2 archetype detection with connectivity scoring and recommendations).
- **Capability registry** (`gang-core`): Content-addressed registry with publish, search (by name/description/tags), install, multi-version support, persist/reload. CLI commands: `gang registry search/install/publish/list/info`.
- **Community pathway**: `docs/CAPABILITY_AUTHOR_GUIDE.md` with language-specific guides for Rust, C++, Python, and Go/TinyGo. `gang capability scaffold <name> --language <lang>` generates project skeletons with Makefile, source template, and WIT directory.
- **Manifest schema v2.0**: Adds authoring language, description, tags, minimum Ganglion version, and schema version fields. v1.x manifests load via `#[serde(default)]` backward compatibility.
- **WIT interface v0.4.0** with `process-spawn`, `network-probe`, and `metrics-emit` interfaces.
- **56 new tests** across 6 crates. **126 total tests passing.**

### Changed

- `CapabilityGroup` enum gains `ProcessSpawn`, `NetworkProbe`, `MetricsEmit` variants.
- `BrokerOperation` enum gains process, network probe, and metric operations.
- Policy engine and permissive policy updated for all new capability groups.

### Breaking

- `ComponentManifest` struct has new required fields (schema_version, language, description, tags, min_ganglion_version) — all have `#[serde(default)]` for backward compatibility with v1.x manifests.

---

## [0.3.0] — 2026-04-23

### Added

- **Content-addressed artifact store** (`gang-core`): `ArtifactStore` with CIDv1 + Blake3 hashing, content-addressed filesystem layout (blobs/ and chunks/ directories with 4-char fanout), configurable chunk size (default 1 MB), block-level deduplication, LRU eviction with configurable size cap (default 1 GB), and JSON metadata index with persist/reload.
- **`Cid` type** (`gang-core`): Content identifier with `bafy` prefix + Blake3 hex hash. Supports `from_bytes`, `from_file`, `from_str`, and `verify`.
- **`ArtifactsPublish` capability group** (`gang-core`): New `CapabilityGroup::ArtifactsPublish` variant for content-addressed artifact publishing.
- **Broker operations** (`gang-core`): `ArtifactPublish` and `ArtifactExists` operations added to `BrokerOperation` enum.
- **WIT interface** (`gang-wasm-host`): `artifacts-publish` interface with `publish` and `exists` functions, WIT updated to v0.3.0.
- **CLI commands** (`gang-cli`): `gang fetch <cid>`, `gang push <path>`, `gang artifacts` for artifact management.
- **Policy engine** updated to handle `ArtifactsPublish` capability group.
- **10 new tests** for artifact store (CID determinism, dedup, chunking, LRU eviction, persist/reload, list). **70 total tests passing.**

---

## [0.2.0] — 2026-04-23

### Added

- **Happy-eyeballs transport selection** (`gang-core`): `TransportPreference` configuration with preferred transport order, dial timeout, and stagger delay. `dial_parallel` method on `TransportAdapter` attempts multiple transports concurrently with first-handshake-wins semantics.
- **Transport statistics** (`gang-core`, `gang-libp2p`): Per-peer `TransportStats` with transport type, RTT tracking, bytes sent/received, DCUtR upgrade status, uptime, and reconnection count. Ping events update RTT in real-time; DCUtR events update relay-to-direct transition state.
- **Network archetype detection** (`gang-ros`): Six network probes (internet connectivity, NAT status, multicast, outbound ports, DNS behavior, CGNAT detection) with classification logic mapping to five standard archetypes. Transport recommendations per archetype.
- **CLI commands**: `gang diagnose [robot]` for network archetype detection with recommendations. `gang transport-stats <robot>` for per-transport connection telemetry. `--prefer-transport` flag on `gang connect` for happy-eyeballs preference.
- **8 new tests** for archetype classification and probe execution (60 total).

### Changed

- `PeerConnection` in libp2p adapter tracks transport type, RTT history, DCUtR state, I/O counters, and reconnection count.

### Breaking

- `TransportAdapter` trait gains `dial_parallel` and `transport_stats` methods (with default implementations — existing impls compile unchanged).

---

## [0.1.0] — 2026-04-23

### Added

- **Core types** (`gang-core`): Ed25519 identity with PeerId derivation, CBOR message framing with varint length prefix, signed component manifests with trust store, default-deny policy engine with glob patterns, append-only audit logging with rotation.
- **libp2p transport** (`gang-libp2p`): Transport adapter with TCP, QUIC, Noise encryption, Yamux multiplexing, circuit relay v2, DCUtR hole-punching, Kademlia peer routing.
- **ROS 2 integration** (`gang-ros`): Diagnostics broker (system info, processes, network state), filesystem broker with symlink jail, log stream broker with source pattern filtering, ROS interface broker (topic subscribe, service call, param get).
- **Robot agent** (`gang-ros`): Deploy/invoke lifecycle with signature verification, trust store checking, policy evaluation, and audit logging.
- **CLI** (`gang`): Full command set — identity, sign, agent, deploy, run, caps, logs, demo, test-archetype, list, connect. Self-contained `gang demo` for zero-dependency end-to-end demonstration.
- **WASM runtime** (`gang-wasm-host`): Wasmtime component model with fuel metering, epoch-based wall-clock deadlines, capability declaration enforcement, and WIT interface definitions for all four v0.1 capability groups.
- **Test harness**: Four Docker-compose scenarios simulating open warehouse, NAT'd office, enterprise DMZ, and mobile/CGNAT network archetypes with tc/netem and iptables.
- **Documentation**: Design specification, implementation plan, quickstart guide, validation framework.
- **52 passing tests** across gang-core (23), gang-ros (19), and gang-wasm-host (10).
