# ADR-022: Robot→operator event subscription layer

**Status:** Accepted; implemented. **Default transport superseded by
[ADR-024](ADR-024-event-push-stream.md)** — the feed now defaults to a genuine
push substream over `libp2p-stream`. The ~1.5 s poll-multiplex described below
is **retained as a selectable fallback** (`events_transport = poll`, and the
target of `auto`'s automatic fallback), so it still matters. The wire model
(`AgentEvent`), the subscriber trust rule, and the bounded resource model
documented here are unchanged and remain in force.
**Date:** 2026-08-06

> **Implementation status.** Landed. A versioned `AgentEvent` wire model
> (`gang_core::events`) plus a bounded robot-side event bus (`gang_ros::events`)
> feed an authenticated event subscription. Emission is wired at the real sites:
> `PolicyDecision` at every deploy `policy.evaluate` (allow and deny) and at
> sandbox policy-denied invokes, `AuditAppended` on every audit append, a
> `PresenceSnapshot` per subscription, a periodic `Heartbeat`, and
> `ConnectionChanged` bridged from transport events. The operator side
> (`Libp2pTransportAdapter::subscribe_events`) opens the feed and decodes a
> framed `AgentEvent` batch. `gang logs`, `gang connect`, `gang transport-stats`,
> and `gang list` are now real (their `[WIP]` markers are removed). Coverage:
> unit tests for the bounded bus (lag→`Gap`, ring eviction→`Gap`, subscriber
> auth) and an in-process integration test over a **real relay circuit**
> (authorized subscribe → snapshot; deny → `PolicyDecision{Deny}`; deploy+invoke
> → `AuditAppended`; unauthorized subscribe refused).

## Context

Ganglion could reach a robot behind NAT and run signed, policy-bounded, audited
tooling on it over a relay circuit (ADR-020), but every operator command was a
one-shot request/response: deploy, invoke, list. There was no way to *observe* a
robot — no "what is this robot doing right now", no live audit tail, no presence.
`gang logs`, `gang connect`, `gang list`, and `gang transport-stats` were honest
stubs waiting on a "presence/streaming layer". That layer is the foundation the
upcoming `gang tui` dashboard will render.

We need an **authenticated, bounded, long-lived event feed** from robot to
operator carrying presence, policy decisions, audit appends, connection changes,
and heartbeats — feeding `gang logs`/`connect` today and the TUI next.

### The transport reality we had to design within

Two hard constraints shaped the design:

1. **No genuine long-lived substream is available without a new dependency.**
   The transport's `GanglionStream` / `TransportAdapter::listen(StreamHandler)`
   look like long-lived substreams, but in the libp2p adapter they are backed by
   `libp2p::request_response` — a single request → single response, with the
   handler's whole output buffered and delivered at stream close. A true push
   substream would need `libp2p-stream`, which the libp2p 0.56 meta-crate does
   not expose as a feature and which is **not in the workspace dependency
   table**. The workspace rule is "no new deps unless already in the table", so a
   push substream is out of scope here.

2. **The shared request-response behaviour always negotiates the first
   protocol.** libp2p `request_response` offers all registered protocols on the
   outbound substream and multistream-select picks the first mutually supported
   one — always `/ganglion/control/1.0`. A distinct `/ganglion/events/1.0`
   protocol id can be registered and served, but the current request-response
   client can never *select* it for an outbound request.

## Decision

### Wire model (`gang_core::events`, dependency-free)

A versioned, `#[non_exhaustive]` `AgentEvent` enum, framed with the **existing**
control-message codec (`encode_message`/`decode_message` — CBOR + varint length
prefix; `encode_events`/`decode_events` are thin loops over it, not a second
codec):

- `PresenceSnapshot { seq, ganglion_version, uptime_secs, archetype,
  installed_capabilities }` — sent once at the head of a fresh subscription.
- `PolicyDecision { seq, ts, operator_peer, capability_group, decision:
  Allow|Deny, reason }` — every policy evaluation, both paths.
- `AuditAppended { seq, record: AuditProjection }` — a secret-free projection of
  the appended `AuditRecord`.
- `ConnectionChanged { seq, ts, peer, transport, via_relay, state: Up|Down }`.
- `Heartbeat { seq, ts, uptime_secs }`.
- `Gap { dropped }` — a synthetic marker inserted when a subscriber fell behind
  the retained window; it carries no sequence of its own.

Every streamed variant carries a monotonic `seq` so a subscriber can resume with
`EventSubscribeRequest { since_seq }`. **No variant carries secret material** —
no private keys, pairing-token secrets, component bytes, or captured tool output.
A unit test asserts the CBOR of a representative event contains none of a set of
secret markers.

`/ganglion/events/1.0` is added to `ALL_PROTOCOLS` and **reserved** for a future
direct-substream client (and served by a registered handler in `serve`). Given
constraint (2), today's subscription rides `/ganglion/control/1.0` as typed
`ControlMessage::SubscribeEvents { since_seq, max_events }` →
`ControlMessage::Events { events }`. The `AgentEvent` schema, the robot-side bus,
and the trust check are identical to the reserved substream path, so switching to
a push substream later is additive.

### Robot side (`gang_ros::events` + `gang_ros::agent`)

A bounded event bus, `EventBus`, with two hard-bounded structures:

- A `tokio::sync::broadcast` channel (capacity **256**) for genuine live
  consumers (the future `gang tui` / push path). A consumer that falls behind
  does not grow robot memory: the channel drops the oldest items and the next
  receive surfaces `Gap { dropped }`, then resumes. `tokio::sync::broadcast` is
  part of tokio (already in the table) — **no new dependency**.
- A `VecDeque` ring (capacity **256**) of recent events, so a request-response
  subscriber can fetch "recent context" and resume by `seq`. A cursor older than
  the retained window yields a `Gap` rather than silently skipping events.

Nothing on this path is unbounded; emission is non-blocking and lock-scoped.

Emission is wired at the real sites: `PolicyDecision` at the deploy
`policy.evaluate` call (allow and deny) and at sandbox policy-denied invokes;
`AuditAppended` on every `record_audit` append; `PresenceSnapshot` built per
subscription; `Heartbeat` every 15 s; `ConnectionChanged` bridged from the
transport's `events()` stream.

### Subscription authentication and resource model

`RobotAgent::build_event_subscription(subscriber, req)` enforces the **same trust
rule as deploy (SEC-03)**: when a trust store is configured, only trusted
operators may subscribe; an empty trust store is the loud dev-permissive path.
The subscriber identity is `remote_peer` — the Ed25519 id libp2p's Noise
handshake authenticated on the stream, never a self-report. An unauthorized peer
is **never streamed to** (it receives an explicit error, no snapshot, no events).
Because the request is an idempotent read, it carries no replay nonce and skips
the replay guard.

### Operator side + CLI

`Libp2pTransportAdapter::subscribe_events(peer, since_seq, timeout)` sends the
subscription and decodes the framed `AgentEvent` batch with bounded local
buffering. The CLI commands reuse the existing circuit-dial path
(`establish_remote_connection` / `connect_via_circuit`, factored out of
`remote_dispatch` — no forked swarm or crypto):

- `gang logs <robot> [--follow] [--since <dur>] [--format json]` — prints
  `AuditAppended` + `PolicyDecision` as human lines or JSONL; `--follow` tails by
  re-polling with an advancing cursor.
- `gang connect <robot>` — a live scrolling status view (presence + heartbeat +
  connection + policy/audit tail); Ctrl-C detaches. The non-TUI precursor to the
  dashboard, reusing the same subscription API.
- `gang transport-stats <robot>` — real per-connection counters from the live
  circuit (the operator transport's `transport_stats`); the simulated data is
  gone.
- `gang list` — registered robots with live reachability from a quick presence
  probe over each peer's relay circuit.

Because the feed rides request-response, a live tail is a **bounded poll**
(default 1.5 s) rather than a persistent push; a genuine push stream is the
reserved substream path above. This is called out in the CLI help and docs
rather than dressed up as a push channel.

## Consequences

- `gang logs`/`connect`/`transport-stats`/`list` are real; the Fleet-status box,
  WIP list, and `CLI_REFERENCE` are updated accordingly.
- The `gang tui` dashboard can build directly on `subscribe_events` and the
  `AgentEvent` schema without re-plumbing.
- **Trade-off:** the live tail polls rather than pushes, and the distinct
  `/ganglion/events/1.0` substream is reserved, not yet the active wire path.
  Both are consequences of the "no new deps" rule against libp2p 0.56's
  request-response-only surface, and both are documented rather than hidden.
- **Security:** an unauthorized peer cannot subscribe (tested); events carry no
  secret material (tested); a slow/stalled consumer cannot grow robot memory —
  broadcast lag and ring eviction both degrade to a `Gap` marker (tested).
