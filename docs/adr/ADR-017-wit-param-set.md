# ADR-017: Add param-set to WIT ros-interface

**Status:** Proposed
**Date:** 2026-04-23

## Context

The WIT `ros-interface` defines `param-get` but not `param-set`:

```wit
interface ros-interface {
    param-get: func(name: string) -> result<list<u8>, string>;
    // no param-set
}
```

Meanwhile, the policy engine supports `RosAccess::ReadWrite` for parameters, implying that write access to parameters is an intended capability. The `gang-capability-param-inspect` crate currently only does read operations (snapshot + diff), but parameter tuning is a common field operation (e.g., adjusting Nav2 costmap parameters, PID gains).

## Decision

Add `param-set` to the WIT `ros-interface` in v0.5:

```wit
param-set: func(name: string, value: list<u8>) -> result<bool, string>;
```

This requires:
1. Adding the function to `ganglion.wit`
2. Adding `BrokerOperation::ParamSet` to `gang-core::broker`
3. Implementing the broker operation in `RosBroker` via rosbridge
4. Gating behind `RosAccess::ReadWrite` in the policy engine (already supported)
5. Bumping the WIT package version to `@0.5.0`

## Consequences

- **Positive:** Enables parameter tuning capabilities — a high-value field operation for deployed robots.
- **Positive:** Completes the read/write symmetry implied by `RosAccess::ReadWrite`.
- **Positive:** The policy engine already distinguishes read-only from read-write; no policy changes needed.
- **Negative:** Parameter writes are a higher-risk operation than reads. A bad parameter value could affect robot behavior. Mitigated by: policy engine requiring explicit `ReadWrite` grant, audit log recording all parameter changes, and manifest signing providing traceability.
- **Negative:** Bumps the WIT package version, which requires existing components to update their declared dependencies to use `param-set`.
