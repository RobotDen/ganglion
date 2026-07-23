# Security Policy

## Reporting a Vulnerability

Please report suspected security vulnerabilities in Ganglion privately by
email to **karma0@gmail.com**. Do not open public GitHub issues for
security reports.

Include as much of the following as you can:

- Affected crate(s) and version(s)
- Network archetype and deployment context (if relevant)
- Reproduction steps or proof of concept
- Impact assessment (what an attacker gains)

## Disclosure Process

We follow **90-day coordinated disclosure**:

1. You report the issue privately; we acknowledge within 5 business days.
2. We investigate, develop, and test a fix.
3. We coordinate a release and advisory with you.
4. After a fix ships — or 90 days after the report, whichever comes first —
   details may be disclosed publicly. We credit reporters unless they prefer
   otherwise.

## Scope

Ganglion's security model (identity, signed manifests, default-deny policy,
WASM sandboxing, audit logging) is documented in the threat model:
[docs/SECURITY.md](../docs/SECURITY.md). Reports that bypass any of those
guarantees are in scope, as are supply-chain issues in the release artifacts.

## Supported Versions

Only the latest released minor version (currently 1.0.x) receives security
fixes.
