# Telemetry checkpoint worker

The **complete server side** of Ganglion's telemetry (ADR-026), published
here so the receiving end is as auditable as the sending end. Read
`worker.js` — it is ~150 lines.

What it guarantees: the client IP is never stored or logged (observability
is disabled in `wrangler.toml`); payloads are schema-validated and dropped
over 4 KiB; the anonymous client id is hashed with a server-side secret
before storage; only aggregate rows land in Workers Analytics Engine, and
raw requests are retained nowhere. Aggregates are kept 13 months.

Deploy (maintainers): `wrangler deploy`, then `wrangler secret put ID_SALT`
with a long random value, and point `checkpoint.robotden.dev` at the worker.
Until it is deployed, clients fail silently by design — nothing about the
CLI changes either way.
