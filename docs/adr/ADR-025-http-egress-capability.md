# ADR-025: `ganglion:http/egress` — URL-pattern-allowlisted HTTP client capability

- Status: Accepted
- Date: 2026-08-19
- Issues: #41 (also enables #42/#43 consumers)

## Context

Capabilities that integrate with HTTP APIs — fleet-adjacent services, device
management consoles, observability endpoints — have no sanctioned path today.
`ganglion:network/probe` is deliberately structured-probe-only (ping, DNS,
port checks; no payload exchange), and reaching HTTP through
`ganglion:process/spawn` + curl would bypass declaration, policy, and audit
entirely. External consumers of the substrate (the Deckhand transition, vault
spec `deckhand-ganglion-transition-strategy`) hit the same wall: every one of
their integrations is an HTTPS API client.

The enforcement claim we want is stronger than a network-level allowlist:
**path- and method-scoped** URL patterns, checked per call, declared in the
signed manifest, gated by the same default-deny policy engine as every other
capability group.

## Decision

Add a ninth capability group, `ganglion:http/egress`, to the
`ganglion:capability@0.5.0` WIT package (interfaces are not independently
versioned).

**Declaration.** A component declares `endpoints`: a list of
`{ pattern, access }` where `pattern` is a URL glob
(`https://api.example.com/v1/**`) and `access` is `read_only` (GET/HEAD
only) or `read_write` (any method). This reuses the existing
`AccessPattern`/access-level shape from `ros/interface`, so the policy
engine's pattern and `max_access` machinery applies verbatim.

**Policy.** `[[capability_rules]]` for the group lists allowed URL patterns;
`max_access = "read_only"` caps every endpoint at GET/HEAD regardless of
declaration. Deploy-time evaluation, `gang policy check` pre-flight, denial
remedies, timed grants (`--until`), history, and lint all work unchanged
because the group rides the existing engine arms.

**Per-call enforcement (two layers).**

1. *Declaration check, host-side (imports layer):* before the broker is
   invoked, the actual request URL and method are validated against the
   calling component's own declared endpoints — the same place the
   declared-group check already happens. A pure, unit-tested helper in
   `gang-core` (`http_request_permitted`) implements the match: exact glob on
   the URL with query string stripped, method gated by the matched pattern's
   access level.
2. *Mechanics check, broker-side:* scheme must be `http`/`https` (pattern
   decides which; anything else refused), response body capped (256 KiB
   default), total deadline capped (10 s default), redirects **not
   followed** (3xx returned to the component — following one could cross the
   allowlist), request body and header sizes bounded, `Host`/`Content-Length`
   header injection refused. TLS terminates host-side via the broker's HTTP
   client (rustls); the component never holds a socket.

**Auditability.** Broker calls flow through the existing
`CapabilityIoStats` accounting; a per-call URL denial is a structured broker
error to the component (the same shape as an undeclared-capability trap), not
a silent failure.

**Query strings** are stripped before pattern matching: patterns govern
*where* a request may go (origin + path), not which parameters it carries.
A pattern cannot be written that grants by query content.

## Consequences

- Components can integrate with exactly the API surface they declared and
  policy permitted — provably nothing else, enforced per call.
- Credentials for these APIs arrive via credential slots (#43), never inside
  the manifest or policy file; the component sets its own `Authorization`
  header from the injected value.
- The broker adds one dependency to `gang-ros` (a small rustls-based HTTP
  client); the CLI's existing curl-shell webhook path in `gang alert` is
  unaffected (operator-side convenience, not sandbox egress).
- `sneakernet`/regulated-facility deployments simply never grant the group.

## Rejected alternatives

- **Widen `network/probe`** — probe results are structured records by design;
  mixing payload exchange into it would blur a clean read-only surface.
- **CIDR/host allowlists** — weaker claim (no path/method scoping) and
  double-resolves DNS (TOCTOU); URL patterns match what the operator reviews.
- **Follow redirects within the allowlist** — every hop re-checked sounds
  safe but makes the effective grant depend on remote configuration; refusing
  redirects keeps the grant static and reviewable.
- **Raw sockets via WASI** — abandons the no-ambient-authority model.
