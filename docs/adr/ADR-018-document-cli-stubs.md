# ADR-018: Document CLI stubbed commands

**Status:** Proposed
**Date:** 2026-04-23

## Context

Several `gang` CLI commands are defined in the `Commands` enum but return placeholder messages or require relay connectivity that isn't wired yet:

- `gang logs` — requires relay connectivity to stream logs from remote robots
- `gang list` — requires relay connectivity to enumerate connected peers
- `gang deploy` — orchestrates component deployment (partially implemented)
- `gang invoke` — invokes a deployed component (partially implemented)

These commands are visible in `gang --help` and documented in `CLI_REFERENCE.md`, but a user attempting them will get confusing "not connected" or stub responses.

## Decision

For v0.5:

1. Mark stubbed commands with `[WIP]` in `--help` output and `CLI_REFERENCE.md`
2. Return a clear, actionable error message: "This command requires relay connectivity (not yet available in v0.4). See docs/QUICKSTART.md for current capabilities."
3. Add a `gang status` command that reports which capabilities are available in the current build

This is preferred over hiding the commands because:
- The commands represent real planned functionality, not abandoned features
- Users reading the architecture docs should see the full intended CLI surface
- Hiding commands that appear in documentation creates confusion

## Consequences

- **Positive:** Users get clear feedback instead of confusing errors.
- **Positive:** The full CLI surface is visible, giving users a picture of the project's direction.
- **Negative:** WIP commands in `--help` may give a "not ready" impression. Mitigated by the `gang demo` command that shows working end-to-end functionality.
