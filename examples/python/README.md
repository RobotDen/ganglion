# Log Normalize — Python Reference Capability

Python implementation of `gang-capability-log-normalize` using
[componentize-py](https://github.com/bytecodealliance/componentize-py).

This demonstrates the Python capability authoring pathway for Ganglion.
The canonical logic lives in the Rust crate at
`crates/gang-capability-log-normalize/`; this example shows the same
algorithm authored in Python.

## Prerequisites

```bash
pip install componentize-py
```

## Build

```bash
# Copy WIT definitions
mkdir -p wit
cp ../../crates/gang-wasm-host/wit/ganglion.wit wit/

# Build WASM component
make component

# Sign and deploy
make sign
gang deploy robot-42 log-normalize.component.wasm
gang run robot-42 log-normalize
```

## Test (native Python, no WASM)

```bash
make test
```

## How it works

The `GanglionCapability` class in `app.py` implements the
`ganglion-capability` WIT world. When built as a WASM component via
`componentize-py`, it can import Ganglion host interfaces (e.g.,
`logs-stream`) and is called by the robot agent's runtime.

See `docs/CAPABILITY_AUTHOR_GUIDE.md` for the full authoring guide.
