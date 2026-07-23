# Multi-Language Capability Examples

Reference implementations of Ganglion capabilities in Python, C++, and Go,
demonstrating that the WASM Component Model authoring pathway works across
multiple languages.

Each example implements the same algorithm as its corresponding Rust crate
in `crates/`, but using the language-specific toolchain:

| Directory | Capability | Language | Toolchain |
|-----------|-----------|----------|-----------|
| `python/` | log-normalize | Python | componentize-py |
| `cpp/` | topic-echo | C++ | wasi-sdk + wit-bindgen |
| `go/` | canary-probe | Go | TinyGo (component build WIP — see `go/README.md`) |

## Prerequisites

Each example has its own prerequisites. See the README in each directory.
All examples require:

1. The Ganglion WIT file — copy from `../crates/gang-wasm-host/wit/ganglion.wit`
   into each example's `wit/` directory.
2. `gang` CLI — for signing and deploying the built components.
3. `wasm-tools` — `cargo install wasm-tools` (except Python, which uses
   componentize-py directly).

## Native testing

Each example can be tested natively without any WASM tooling:

```bash
cd python && make test
cd cpp && make test
cd go && make test
```

## Building WASM components

Each example's `make component` builds a WASM component ready for signing
and deployment. This requires the language-specific WASM toolchain:

- **Python**: `pip install componentize-py`
- **C++**: wasi-sdk v26+ (`export WASI_SDK_PATH=...`)
- **Go**: TinyGo 0.34.0+

See `../docs/CAPABILITY_AUTHOR_GUIDE.md` for complete authoring instructions.
