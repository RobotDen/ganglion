# Canary Probe — Go Reference Capability

Go implementation of `gang-capability-canary-probe` using
[TinyGo](https://tinygo.org/) and the WASM Component Model.

This demonstrates the Go capability authoring pathway for Ganglion,
proving the platform supports a fourth language beyond Rust, C++,
and Python.

> **WIP:** The Go component build (`make component` via TinyGo) is
> experimental. TinyGo's `wasip2` target and component tooling are still
> maturing, and the WIT bindings/WASI-adapter step may need adjustment for your
> TinyGo version (see the note in the authoring guide). The native `make test`
> path works today and is the reliable way to exercise the logic.

The canonical logic lives in the Rust crate at
`crates/gang-capability-canary-probe/`.

## Prerequisites

```bash
# TinyGo 0.34.0+
# See https://tinygo.org/getting-started/install/

# wasm-tools
cargo install wasm-tools
```

## Build

```bash
# Copy WIT definitions
mkdir -p wit
cp ../../crates/gang-wasm-host/wit/ganglion.wit wit/

# Build WASM component (requires TinyGo)
make component

# Sign and deploy
gang sign canary-probe.component.wasm --name canary-probe --version 0.1.0
gang deploy robot-42 canary-probe.component.wasm
gang run robot-42 canary-probe
```

## Test (standard Go, no TinyGo)

```bash
make test
```

This runs the health check algorithm natively with sample data.

## How it works

`main.go` implements the canary probe health check algorithm with
configurable thresholds for memory, disk, uptime, and reachability.
When built as a WASM component via TinyGo, it calls the
`diagnostics-collect` and `network-probe` host imports to gather
real system data.

The `main()` function runs with sample data for testing. In the
WASM component, the `Run()` function registered via `init()` serves
as the entry point.

See `../../docs/CAPABILITY_AUTHOR_GUIDE.md` for the full authoring guide.
