# ADR-010: Content-addressed artifact store (Blake3 + CIDv1)

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at v0.3)

## Context

WASM components, diagnostic bundles, and log snapshots need to be stored and transferred between peers. Requirements:

- Deduplication: the same artifact stored twice should not double storage
- Integrity: content must be verifiable without trusting the source
- Addressability: artifacts should be retrievable by content, not location
- Eviction: edge devices have limited storage; old artifacts must be purgeable

Options for content addressing:

1. **SHA-256 with custom envelope:** Standard hash, but slow on edge hardware and no ecosystem for content identifiers.
2. **Blake3 with CIDv1 (IPFS-compatible):** Fast (SIMD-accelerated), parallelizable, produces IPFS-compatible content identifiers. Enables future interop with IPFS pinning services.
3. **Blake2b:** Used by early IPFS. Superseded by Blake3 in performance.

## Decision

Use Blake3 for content hashing. Wrap hashes in CIDv1 format (multicodec + multihash) for interoperability. The artifact store supports:

- Deterministic CID generation (same content → same CID)
- Deduplication (store returns existing CID if content matches)
- Chunking for large artifacts
- LRU eviction with configurable capacity
- Persist/reload to JSON for durability across restarts

## Consequences

- **Positive:** Blake3 is 2-5x faster than SHA-256 on typical edge hardware. Hashing is not a bottleneck.
- **Positive:** CIDv1 format enables future IPFS/Filecoin integration for off-robot archival.
- **Positive:** LRU eviction prevents unbounded storage growth without manual cleanup.
- **Negative:** CIDv1 encoding adds complexity compared to raw hex hashes. Justified by interoperability benefits.
- **Negative:** JSON persistence for the artifact store metadata is simple but not crash-safe. A future version should use a write-ahead approach or SQLite.
