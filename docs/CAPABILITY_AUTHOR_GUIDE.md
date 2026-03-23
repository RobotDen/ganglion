# Capability Author Guide

Build, sign, and distribute Ganglion capabilities in Rust, C++, Python, or Go.

## Overview

A Ganglion capability is a WASM component that runs inside the sandboxed Layer 2 runtime on a robot. Capabilities interact with the robot through well-defined WIT interfaces — they cannot access resources they haven't declared, and the policy engine enforces boundaries at load time.

## Quick Start (Rust)

```bash
# Scaffold a new capability
gang capability scaffold my-diagnostics --language rust

# Build
cd my-diagnostics
cargo build --target wasm32-wasip2 --release

# Create component from module
wasm-tools component new target/wasm32-wasip2/release/my_diagnostics.wasm \
  -o my-diagnostics.component.wasm

# Sign with your identity
gang sign my-diagnostics.component.wasm --name my-diagnostics --version 0.1.0

# Test locally
gang deploy localhost my-diagnostics.component.wasm
gang run localhost my-diagnostics

# Publish to registry
gang registry publish my-diagnostics.component.wasm \
  --description "Custom diagnostics for my robot" \
  --tags diagnostics,custom
```

## Architecture

```
Operator                          Robot
  |                                 |
  |-- deploy(component.wasm) ------>|
  |                                 | verify signature
  |                                 | check trust store
  |                                 | evaluate policy
  |                                 | instantiate WASM
  |-- run(capability, args) ------->|
  |                                 | fuel metering
  |                                 | epoch deadline
  |<-- result / stream ------------|
```

### Capability lifecycle

1. **Author** writes capability code targeting WIT interfaces
2. **Build** compiles to `wasm32-wasip2` target
3. **Component** creation wraps the module as a WASM component
4. **Sign** with Ed25519 identity produces `<name>.manifest.cbor`
5. **Deploy** sends component + manifest to robot
6. **Verify** — robot checks signature, trust store, policy
7. **Run** — WASM runtime instantiates with declared interfaces only

## WIT Interfaces

Your capability declares which interfaces it needs. Available groups:

| Interface | WIT Name | What it provides |
|-----------|----------|-----------------|
| ROS Interface | `ganglion:ros/interface` | Topic subscribe, service call, param get |
| Log Stream | `ganglion:logs/stream` | Log source enumeration, filtered streaming |
| Filesystem | `ganglion:fs/bounded` | Path-gated file read/write/list |
| Diagnostics | `ganglion:diagnostics/collect` | System info, process list, network state |
| Artifacts | `ganglion:artifacts/publish` | Content-addressed data publishing |
| Process | `ganglion:process/spawn` | Bounded subprocess invocation |
| Network Probe | `ganglion:network/probe` | Ping, DNS, port check, traceroute |
| Metrics | `ganglion:metrics/emit` | Structured metric emission |

### Entry point

Every capability exports a single function:

```wit
export run: func(args: list<string>) -> result<list<u8>, string>;
```

`args` are operator-supplied arguments. Return serialized result data on success, or an error string on failure.

## Language-Specific Guides

### Rust

**Prerequisites:** Rust toolchain with `wasm32-wasip2` target.

```bash
rustup target add wasm32-wasip2
cargo install wasm-tools
```

**Project structure:**

```
my-capability/
  Cargo.toml
  src/
    lib.rs          # Capability implementation
  wit/
    ganglion.wit    # Copy from ganglion repo
```

**Cargo.toml:**

```toml
[package]
name = "my-capability"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

**src/lib.rs:**

```rust
// WIT bindings would be generated here via wit-bindgen
// For now, this shows the pattern:

use serde::Serialize;

#[derive(Serialize)]
struct DiagResult {
    hostname: String,
    status: String,
}

// Entry point — called by the Ganglion runtime
pub fn run(args: Vec<String>) -> Result<Vec<u8>, String> {
    let result = DiagResult {
        hostname: "robot-01".into(),
        status: "healthy".into(),
    };
    serde_json::to_vec(&result).map_err(|e| e.to_string())
}
```

**Build:**

```bash
cargo build --target wasm32-wasip2 --release
wasm-tools component new target/wasm32-wasip2/release/my_capability.wasm \
  -o my-capability.component.wasm
```

### C++

**Prerequisites:** wasi-sdk, wit-bindgen for C++.

```bash
# Install wasi-sdk
curl -LO https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-25/wasi-sdk-25.0-x86_64-linux.tar.gz
tar xf wasi-sdk-25.0-x86_64-linux.tar.gz
export WASI_SDK_PATH=$(pwd)/wasi-sdk-25.0
```

**Project structure:**

```
my-capability/
  Makefile
  src/
    main.cpp
  wit/
    ganglion.wit
```

**Build (Makefile):**

```makefile
WASI_SDK ?= $(WASI_SDK_PATH)
CC = $(WASI_SDK)/bin/clang++

my-capability.wasm: src/main.cpp
	$(CC) -o $@ $< --target=wasm32-wasip2 -O2
	wasm-tools component new $@ -o my-capability.component.wasm
```

### Python

**Prerequisites:** componentize-py.

```bash
pip install componentize-py
```

**Project structure:**

```
my-capability/
  app.py
  wit/
    ganglion.wit
```

**app.py:**

```python
import json

def run(args: list[str]) -> bytes:
    result = {
        "message": "Hello from Python capability",
        "args": args,
    }
    return json.dumps(result).encode()
```

**Build:**

```bash
componentize-py -d wit/ganglion.wit -w ganglion-capability componentize app -o my-capability.component.wasm
```

### Go (TinyGo)

**Prerequisites:** TinyGo with WASI support.

```bash
# Install TinyGo
brew install tinygo  # or see https://tinygo.org/getting-started/install/
```

**Project structure:**

```
my-capability/
  main.go
  go.mod
  wit/
    ganglion.wit
```

**main.go:**

```go
package main

import "encoding/json"

type Result struct {
    Status string `json:"status"`
    Message string `json:"message"`
}

//export run
func run() {
    result := Result{Status: "ok", Message: "Hello from Go"}
    data, _ := json.Marshal(result)
    // Write to WASI stdout
    println(string(data))
}

func main() {}
```

**Build:**

```bash
tinygo build -o my-capability.wasm -target=wasip2 .
wasm-tools component new my-capability.wasm -o my-capability.component.wasm
```

## Manifest and Signing

Every deployed capability requires a signed manifest:

```bash
# Sign the component
gang sign my-capability.component.wasm \
  --name my-capability \
  --version 0.1.0

# This produces: my-capability.manifest.cbor
```

The manifest contains:
- Component name and version
- Declared capability groups (WIT interfaces used)
- Author peer ID
- Ed25519 signature over component bytes + manifest fields
- Optional resource limits (max memory, CPU budget, wall-clock deadline)

## Policy and Security

Capabilities run under **default-deny** policy:

- Only declared WIT interfaces are linked — undeclared imports trap immediately
- Pattern-based access control for ROS topics, filesystem paths, log sources
- Fuel metering prevents runaway CPU usage
- Epoch-based wall-clock deadlines prevent hanging
- Memory limits enforced by the WASM runtime
- Process spawning restricted to command allowlists

The robot operator controls policy. Your capability must declare everything it needs, and the operator's policy must permit it.

## Publishing to the Registry

```bash
# Publish to local registry
gang registry publish my-capability.component.wasm \
  --description "My custom diagnostic capability" \
  --tags diagnostics,custom,ros2

# Others can discover it
gang registry search diagnostics

# And install it
gang registry install my-capability
```

## Best Practices

1. **Declare minimally.** Only request the capability groups you actually need. Fewer declarations = easier policy approval.
2. **Handle errors gracefully.** Return descriptive error strings — the operator sees them.
3. **Respect resource limits.** Design for bounded execution. If you need to process large data, use streaming patterns.
4. **Version your output format.** Include a version field in your serialized output so operators can handle schema evolution.
5. **Test without ROS.** Structure your code so the business logic is testable as a regular library without requiring a ROS 2 environment.
6. **Use content addressing.** For large outputs, publish artifacts via `ganglion:artifacts/publish` and return the CID.
