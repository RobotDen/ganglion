# Topic Echo — C++ Reference Capability

C++ implementation of `gang-capability-topic-echo` using
[wasi-sdk](https://github.com/WebAssembly/wasi-sdk) and
[wit-bindgen](https://github.com/bytecodealliance/wit-bindgen).

This demonstrates the C++ capability authoring pathway for Ganglion.
C++ is the ROS 2 community's primary language; this example shows
native-language parity for capability development.

The canonical logic lives in the Rust crate at
`crates/gang-capability-topic-echo/`.

## Prerequisites

```bash
# wasi-sdk v26+
export WASI_SDK_PATH=/opt/wasi-sdk

# wit-bindgen and wasm-tools
cargo install wit-bindgen-cli wasm-tools
```

## Build

```bash
# Copy WIT definitions
mkdir -p wit
cp ../../crates/gang-wasm-host/wit/ganglion.wit wit/

# Build WASM component (requires wasi-sdk)
make component

# Sign and deploy
gang sign topic-echo.component.wasm --name topic-echo --version 0.1.0
gang deploy robot-42 topic-echo.component.wasm
gang run robot-42 topic-echo -- /odom --decimation 5
```

## Test (native C++, no WASM)

```bash
make test
```

This compiles and runs the decimation algorithm natively, without
wasi-sdk or WASM tooling.

## How it works

`src/main.cpp` implements the topic echo decimation algorithm. When
built with wit-bindgen, it calls the `ros-interface::topic-subscribe`
host import to receive messages, applies decimation, and returns the
captured messages as JSON.

The `STANDALONE_TEST` build mode demonstrates the algorithm without
WIT bindings by running sample data through the decimation logic.

See `docs/CAPABILITY_AUTHOR_GUIDE.md` for the full authoring guide.
