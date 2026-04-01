# ADR-011: Manifest schema v2 with backward compatibility

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.3)

## Context

v0.1 introduced signed component manifests with a minimal schema (name, version, capabilities, hash, signature). v0.3 added new fields:

- `artifacts`: list of content-addressed artifact references
- `schema_version`: explicit version tracking for the manifest format itself

Existing v0.1 manifests in the wild (test fixtures, deployed components) lack these fields. Breaking deserialization would prevent upgrades.

## Decision

Bump the manifest schema to v2.0 (`MANIFEST_SCHEMA_VERSION = "2.0"`). New fields use `#[serde(default)]` so that v1.x manifests deserialize successfully with default values:

- `artifacts` defaults to empty vec
- `schema_version` defaults to `"1.0"` via `default_schema_version()` (not `"2.0"`) — this is intentional: a manifest without an explicit schema version is a v1.x manifest

The `default_schema_version()` function returning `"1.0"` is deliberate backward-compatibility behavior, not a bug.

## Consequences

- **Positive:** Zero-downtime upgrade path. Existing manifests continue to work.
- **Positive:** New manifests are self-describing via `schema_version`.
- **Positive:** Signature verification works unchanged — the signature covers the serialized bytes, which include default values after deserialization.
- **Negative:** Code must handle both v1 and v2 manifests indefinitely, or a migration tool must be provided.
- **Negative:** The `default_schema_version() → "1.0"` pattern is surprising to readers who expect it to return the current version. Documented with a comment in `manifest.rs`.
