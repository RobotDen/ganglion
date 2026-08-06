# ADR-024: True server-push event stream (with poll fallback) over libp2p-stream

**Status:** Accepted; implemented
**Date:** 2026-08-06
**Supersedes:** the *default* transport mechanism of [ADR-022](ADR-022-event-subscription-layer.md) (poll-multiplex over the control protocol). The poll path is **retained as a selectable fallback**, not removed. The wire model, trust rule, and bounded resource model of ADR-022 are unchanged.

> **Implementation status.** Landed. The robot→operator event feed defaults to a
> genuine persistent push substream on `/ganglion/events/1.0`, carried by
> `libp2p-stream`. The robot accepts inbound event substreams, authenticates
> each subscriber, sends the `PresenceSnapshot` + retained catch-up, then pushes
> framed `AgentEvent`s live from the bounded `EventBus` broadcast until the
> stream closes. It also still serves the `ControlMessage::SubscribeEvents` poll
> on the control protocol. The operator's `subscribe_events` returns a live
> `Stream<AgentEvent>` (an [`EventFeed`]) carried by EITHER transport, chosen by
> an `events_transport` selector (`auto`/`push`/`poll`, default `auto`) with a
> per-command CLI flag override. `auto` prefers push and falls back to poll when
> push is unavailable (or drops mid-session). Measured push latency over a real
> relay circuit: ~2 ms (asserted `< 500 ms` in `tests/event_subscription.rs`),
> versus the ~1.5 s poll cadence.
>
> **Why keep the poll?** `libp2p-stream` is a pre-release (0.4.0-alpha). Keeping
> the request-response poll — which shipped in ADR-022 and needs no new
> dependency — means an operator is never left without a feed if the alpha
> misbehaves, if it is disabled, or if the peer runs an older/alpha-free agent.
> `auto` de-risks the alpha transparently; `push`/`poll` force a path for
> debugging or policy.

## Context

ADR-022 shipped an authenticated, bounded robot→operator event feed (presence,
policy decisions, audit appends, connection changes, heartbeats) but had to
carry it as a **bounded ~1.5 s poll**: each tail iteration sent a typed
`ControlMessage::SubscribeEvents { since_seq }` over the request-response
`/ganglion/control/1.0` protocol and received a buffered `Events` batch back.

That was a stopgap forced by two constraints documented in ADR-022:

1. libp2p's request-response behaviour buffers the whole handler output and
   delivers it at stream close — a single request → single response, not a
   long-lived push.
2. The libp2p 0.56 meta-crate exposes **no `stream` feature**, and the workspace
   rule is "no new deps unless already in the table", so a genuine push
   substream was out of scope. `/ganglion/events/1.0` was reserved but unused as
   a wire path; the poll rode control instead.

The consequences were visible to operators: `gang logs --follow`, `gang
connect`, and `gang tui` updated on a ~1.5 s cadence, and every tail iteration
re-opened a control RPC and re-sent a cursor. The reserved `/ganglion/events/1.0`
substream was the intended end state all along.

## Decision

Add exactly **one** new dependency — `libp2p-stream` — and build the reserved
push substream.

### Dependency: `libp2p-stream 0.4.0-alpha`

`cargo add libp2p-stream -p gang-libp2p --dry-run` resolves **v0.4.0-alpha**
cleanly against the locked `libp2p-swarm 0.47.1` (the same swarm crate the
libp2p 0.56 meta-crate pulls in), so the `libp2p_stream::Behaviour`,
`libp2p::Stream`, and `libp2p::StreamProtocol` types unify with the existing
graph — no second swarm, no version skew.

- It is added to `[workspace.dependencies]` and to `gang-libp2p` only.
- It is **pinned exactly** (`=0.4.0-alpha`) because it is a pre-release: a future
  `0.4.0` final or a new alpha must not silently change the wire behaviour
  underneath us. Bumping it is a deliberate, reviewed change.
- **Risk:** it is a pre-release of a small, well-scoped crate (behaviour +
  control handle + upgrade). `cargo deny check` passes clean
  (`advisories ok, bans ok, licenses ok, sources ok`) — the alpha is **not**
  flagged by any advisory or ban, so **no `deny.toml` exception is required**.
  If a future advisory or ban flags it, add a minimal, commented per-crate
  exception rather than loosening the policy globally.

### Wiring into the single-owner swarm

`libp2p_stream::Behaviour` is added to `GanglionBehaviour` (the derived
`NetworkBehaviour`). The swarm stays owned by exactly one task (the
`SwarmWorker`); we do **not** reintroduce a shared `Swarm` mutex. Instead, at
build time `build_swarm` detaches a cloneable `libp2p_stream::Control` handle
(`behaviour.stream.new_control()`) and hands it to the adapter. `Control` talks
to the behaviour over its own internal channel, driven whenever the worker polls
`swarm.next()` — so opening and accepting streams never touch the `Swarm`
directly. `/ganglion/events/1.0` is removed from the request-response protocol
set so only the stream behaviour claims that inbound protocol id.

### Robot side (accept + push)

`RobotAgent::serve` calls `Libp2pTransportAdapter::accept_event_streams()`,
which registers `Control::accept("/ganglion/events/1.0")` and yields inbound
`(subscriber, substream)` pairs. The subscriber id is the SEC-03 gang id derived
from the Ed25519 key libp2p's Noise handshake authenticated on the connection;
a peer whose identity cannot be recovered is dropped before it is surfaced. Per
subscriber, one task: read the single `EventSubscribeRequest` frame, then reuse
the **existing** `build_event_subscription` (same trust rule as deploy — trusted
-only when a trust store is configured, loud dev-permissive when empty) to
authenticate and build the snapshot + retained catch-up. An unauthorized peer is
streamed **nothing** (the substream is dropped). Otherwise the task pushes the
catch-up batch and then forwards live events from the bounded `EventBus`
broadcast until the stream closes. The live subscription is taken **before** the
snapshot so no event is lost in the window, and catch-up/live overlap is
deduplicated by sequence. A slow/lagging consumer degrades to an
`AgentEvent::Gap` marker via the bounded broadcast — never unbounded robot
memory. No secrets on the wire (the `AgentEvent` model is unchanged).

### Operator side (open + decode)

`Libp2pTransportAdapter::subscribe_events` now returns a live [`EventFeed`] (a
`Stream<AgentEvent>`) instead of a `Vec`. On the **push** path it opens a
`/ganglion/events/1.0` substream (over the relay circuit, same path as control
RPC via `Control::open_stream`), writes one `EventSubscribeRequest` frame, and
reads the first frame within the handshake timeout — a clean EOF there means the
robot refused the subscription (unauthorized), surfaced as a typed
`TransportError`. It then decodes length-prefixed CBOR `AgentEvent`s as they are
pushed. `gang logs`/`connect`/`tui` consume the `Stream` without caring which
transport is live; `EventFeed::active_transport()` reports the current one.

### Transport selection and automatic fallback

The `events_transport` selector (`Libp2pConfig::events_transport`, overridable
per command with `--events-transport`) chooses:

- **`push`** — force the substream; if it cannot be opened, error clearly (never
  a silent poll).
- **`poll`** — never open a stream; run the retained `SubscribeEvents`
  request-response loop on `events_poll_interval_ms` (default 1500 ms).
- **`auto`** (default) — try push; classify the outcome:
  - open fails with `OpenStreamError::UnsupportedProtocol` (multistream
    `NegotiationFailed`), any other open error, or a handshake io/timeout →
    **push unavailable** → fall back to poll and log it.
  - the stream opens but the robot returns a clean EOF on the first frame →
    **refusal** (unauthorized) → surface the error; poll would refuse
    identically, so no fallback.
  - success → push, and if the push substream **drops mid-session** the returned
    `EventFeed` transitions to poll (resuming from the last cursor) so the feed
    is never left dead.

The push-open failure is classified in `open_push` into `Unavailable` / `Fatal`
/ `Refused`; the poll loop is a `Stream` built by `poll_loop`, so both transports
present the identical `Stream<AgentEvent>` shape. An eager first exchange on both
paths makes a refusal surface at `subscribe_events` time rather than as a
silently empty stream.

### Framing

The push feed reuses the **existing** length-prefixed CBOR framing
(`gang_core::message::encode_message`/`decode_message`: `[varint length][CBOR]`).
A small shared async read/write helper (`gang_libp2p::framed`) operates that
framing over the raw `libp2p::Stream` (`futures::AsyncRead + AsyncWrite`) so the
robot push loop and the operator decode loop agree byte-for-byte with the
buffered path. No second codec.

## Consequences

- Operators see events **the instant the robot emits them** (~2 ms over a relay
  circuit in-test) rather than on a ~1.5 s cadence. `gang logs --follow`,
  `gang connect`, and `gang tui` are all live.
- The TUI `♥ live` / `[stale feed]` indicator is still driven by the 15 s
  heartbeats: liveness detection is unchanged; only feed latency changed.
- One new dependency (`libp2p-stream`), pinned to a pre-release, is the cost.
  `cargo deny` is clean, so no policy exception was needed. Because the poll
  fallback is retained, the alpha is never a single point of failure for the
  feed: `auto` degrades to poll transparently, and operators can force `poll` to
  avoid the alpha entirely.
- The single-owner swarm architecture, the `AgentEvent` wire model, the
  subscriber trust rule (SEC-03), and the bounded resource model (broadcast lag
  and ring eviction → `Gap`) are all **unchanged** from ADR-022 — this ADR
  swaps only the transport mechanism from poll-multiplex to true push.
- Coverage: `tests/event_subscription.rs` keeps the ADR-022 assertions
  (authorized subscribe → `PresenceSnapshot`; deny → `PolicyDecision{Deny}`;
  invoke → `AuditAppended` + `PolicyDecision{Allow}`; unauthorized refused) and
  adds, over a real relay circuit: push-latency (`< 500 ms`, forced `push`);
  `poll` mode delivers events; `auto` falls back to poll against a **poll-only
  robot** (`serve_poll_only`, which does not accept the push protocol) and still
  delivers events; and forced `push` against a poll-only robot errors rather
  than silently polling. The broadcast-lag → `Gap` and ring-eviction → `Gap`
  paths remain covered by unit tests in `gang_ros::events`.

[`EventFeed`]: ../../crates/gang-libp2p/src/adapter.rs
