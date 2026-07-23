# ADR-014: Rust edition 2024

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.4)

## Context

Rust edition 2024 (stabilized in Rust 1.85) introduces several language improvements:

- `gen` blocks for iterators
- `async` closures
- Improved `unsafe` ergonomics
- `use<>` precise capturing syntax for `impl Trait`
- Updated formatting defaults in `rustfmt`

Ganglion was started after edition 2024's stabilization. (The MSRV at the time of this decision was 1.85; it has since been raised to 1.88.)

## Decision

Use `edition = "2024"` in all workspace crate `Cargo.toml` files. This is the newest stable edition and aligns with the project's Rust version requirement.

## Consequences

- **Positive:** Access to all edition 2024 language features without `#![feature(...)]` flags.
- **Positive:** `cargo fmt` uses edition 2024 formatting rules by default, producing consistent style.
- **Negative:** Contributors with Rust older than the MSRV (now 1.88) cannot build the project. Documented in README and QUICKSTART.
- **Negative:** Some crate dependencies may not yet be tested with edition 2024 formatting. No issues encountered in practice.
