# ADR-023: `gang tui` live fleet dashboard

**Status:** Accepted; implemented
**Date:** 2026-08-06

> **Implementation status.** Landed. `gang tui` renders a four-pane ratatui
> dashboard (peers, tunnels, policy decisions, audit tail) over the ADR-022
> event subscription layer. The core is a pure event-fold state reducer plus
> `TestBackend` render tests; the event loop is a thin shell. Verified live
> against a `gang up` fleet (allow + deny appear in the policy pane and the
> audit tail). New deps: `ratatui` 0.30, `crossterm` 0.29 (gang-cli only).

## Context

ADR-022 gave us an authenticated, bounded robot→operator event feed and made
`gang logs`/`connect`/`transport-stats`/`list` real. `gang connect` already
tails a single robot as scrolling text. Issue #2 asks for the next step: a live
**dashboard** — the project's best demo-GIF surface — showing connected peers,
active tunnels, live policy allow/deny decisions, and a tailing audit log across
the whole fleet, with a strong UX bar (clarity, pause-for-capture, `NO_COLOR`,
graceful resize).

Two forces shaped the design:

1. **CI has no TTY.** The dashboard's logic must be testable headless, so
   correctness is not gated on a terminal.
2. **The feed was a bounded ~1.5 s poll, not push** (ADR-022, at the time this
   ADR was written). The UI must be honest about staleness rather than fake
   sub-second motion, and must never block the render thread on a slow or dead
   robot. (The feed later became a genuine push substream — see
   [ADR-024](ADR-024-event-push-stream.md); the dashboard consumes it
   unchanged, and its staleness indicator now reflects a dead connection rather
   than a missed poll.)

## Decision

### Split the TUI into a testable core and a thin shell

- **State reducer (`tui::state`).** A pure `DashboardState::apply(FeedMsg, now)`
  folds one tagged feed message into UI rows. It has no I/O and no clock of its
  own — the caller passes `now` — so the whole fleet-view logic (presence →
  peer row + caps, `PolicyDecision` → policy pane, `AuditAppended` → audit pane,
  `ConnectionChanged`/stats → peer status + tunnels, `Gap` → a notice) is unit
  tested by feeding synthetic `AgentEvent`s and asserting the resulting rows.
- **Render (`tui::render`).** `render(frame, state, theme, phase, now)` is a
  pure function onto a `ratatui::Frame`. Every layout — populated, paused,
  inspect/help overlays, first-run panel, too-small fallback, and the
  `NO_COLOR` degrade — is exercised via `render_to_lines` over
  `ratatui::backend::TestBackend`, which returns the rendered buffer as text.
- **Shell (`tui::mod`).** Opens one subscription task per robot on tokio,
  funnels `FeedMsg`s over an mpsc channel, and runs the crossterm input +
  redraw loop in one `tokio::select!`. This is the only part that touches a real
  terminal, and it is deliberately small.

### Liveness keys off successful polls, not just heartbeats

The agent emits a `Heartbeat` only every 15 s, so keying "live" purely off
events would leave a healthy robot flapping to "transitional" between beats.
Instead, a **successful ~1.5 s transport-stats read** refreshes `last_seen`: it
is direct evidence the circuit is alive *now*. A peer is live within 4 s of the
last contact, transitional within 12 s, then offline. The title bar carries a
`♥ live` pulse that flips to `[stale feed]` when no contact arrives within the
poll cadence, so a stalled feed is visible rather than silently frozen.

### Pause buffers rather than drops

`p` freezes the display for a clean capture. Incoming messages are **buffered**
(not dropped) while paused and replayed in order on resume, so the demo picks up
exactly where it left off. A `PAUSED` chip shows in the title bar.

### Theme honors `NO_COLOR` as a first-class mode

A `Theme` struct resolves either the teal accent palette or a monochrome/ASCII
degrade when `NO_COLOR` is set (any non-empty value, per no-color.org). In
monochrome mode borders become ASCII (`+ - |`), status markers become `* ~ .`,
and every decorative glyph (arrows, ellipses, mid-dots) is ASCII, so the whole
frame is pure ASCII with no color escapes — the state a recorder captures. A
headless test asserts the monochrome frame `is_ascii()`.

### Reuse the existing dial path; add no new transport

Subscription tasks reuse `establish_remote_connection` / `connect_via_circuit`
and `subscribe_events` / `transport_stats` exactly as `gang logs`/`connect`/
`transport-stats` do. The TUI introduces **no new transport path** — it is a new
consumer of ADR-022.

## Consequences

- A TUI genuinely needs a terminal library; `ratatui` 0.30 and `crossterm` 0.29
  are added to `[workspace.dependencies]` (majors pinned) and used in `gang-cli`
  only, keeping the library crates dependency-light. `crossterm`'s
  `event-stream` feature lets key input join the tokio select loop without a
  blocking input thread.
- Because the reducer and renderer are pure and headless, CI covers the dashboard
  without a TTY. A `--frames N` snapshot mode renders a frame to stdout as text
  for capture and scripting.
- The terminal is restored (raw mode off, alternate screen left) on normal exit,
  error, and panic (RAII guard + panic hook), so a crash never leaves the
  operator's shell garbled.
- The dashboard is only as fast as the ADR-022 poll. That is intentional and
  surfaced to the user; a future `/ganglion/events/1.0` push substream (already
  reserved) would drop straight in behind the same `FeedMsg` channel.
