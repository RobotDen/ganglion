# ADR-003: Ed25519 identity model

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.1)

## Context

Ganglion needs a stable identity for every participant (robot, operator, relay) that works across network changes, reconnections, and organizational reshuffling. Human-readable names (fleet names, robot hostnames) change; identity must not.

Options considered:

1. **X.509 certificates** — industry standard, but requires a CA infrastructure that operators may not have, and certificate rotation is operationally complex on edge devices.
2. **Ed25519 keypairs with Blake3-derived PeerId** — self-certifying identity, no CA needed, compatible with libp2p's native identity model.
3. **Pre-shared symmetric keys** — simple but doesn't provide per-peer identity or non-repudiation.

## Decision

Use Ed25519 keypairs as the canonical identity. PeerId is derived as `12D3-` + Blake3(public_key)[..16]. Keys are generated locally and stored at `~/.gang/identity.key`. The keypair supports sign/verify for manifest attestation, peer authentication, and audit trail binding.

## Consequences

- **Positive:** Zero infrastructure to bootstrap — `gang identity generate` creates a working identity with no external dependencies.
- **Positive:** Compatible with libp2p's peer identity model, enabling direct use of Kademlia, circuit relay, and DCUtR.
- **Positive:** Sign/verify on manifests provides cryptographic proof of authorship without a PKI.
- **Negative:** No built-in key rotation mechanism. A compromised key requires manual replacement and trust store updates across the fleet.
- **Negative:** No hierarchical trust — all peers are equal. Fleet-level authority delegation (e.g., "this operator can deploy to these robots") requires the policy engine, not the identity layer.
- **Future work:** Key rotation protocol, hardware-backed key storage (TPM/Secure Enclave), and optional integration with organizational CAs for enterprises that require it.
