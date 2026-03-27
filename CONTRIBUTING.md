# Contributing to Ganglion

Contributions are welcome. This document covers the development workflow, code standards, and how to submit changes.

## Getting started

```bash
# Clone the repository
git clone https://github.com/tafy-labs/ganglion.git
cd ganglion

# Prerequisites: Rust 1.85+
rustup update stable

# Set up git hooks
./scripts/setup-hooks.sh

# Build and test
cargo build
cargo test
```

## Development workflow

### Pre-commit hooks

After running `./scripts/setup-hooks.sh`, every commit will automatically run:

1. `cargo fmt --check` — formatting must pass
2. `cargo clippy --all-targets -- -D warnings` — no clippy warnings allowed
3. `cargo test` — all tests must pass

If any check fails, the commit is rejected. Fix the issue and try again.

### Branch conventions

- `main` — stable, tagged releases
- Feature branches — `feature/<name>` or `<name>` off `main`
- All changes go through pull requests

### Pull request process

1. Fork the repository and create a feature branch
2. Make your changes with tests
3. Ensure all pre-commit checks pass locally
4. Push your branch and open a pull request against `main`
5. CI will run the same checks (fmt, clippy, test on Linux + macOS, doc build)
6. Address review feedback
7. Squash-merge when approved

### Commit messages

Use the imperative mood and describe what the commit does, not what you did:

```
Add parameter snapshot diff capability

- Implements BTreeMap-based comparison of ROS 2 parameter snapshots
- Produces Added/Removed/Changed diff entries
- Includes human-readable format_diff() output
```

Keep the first line under 72 characters. Use the body for details.

## Code standards

### Formatting

All code must pass `cargo fmt`. The project uses default rustfmt settings.

### Clippy

All code must pass `cargo clippy --all-targets` with no warnings. CI runs with `RUSTFLAGS="-Dwarnings"`, which promotes all warnings to errors.

### Tests

- Every new module should include tests in a `#[cfg(test)] mod tests` block
- Test both happy paths and error cases
- Use `tempfile::TempDir` for filesystem tests (not `/tmp` directly)
- Network tests should use localhost or mock data, not external services
- Async tests use `#[tokio::test]`

### Documentation

- Public types and functions should have doc comments
- Doc comments should explain *why*, not just *what*
- Run `cargo doc --no-deps` and check for warnings (CI runs with `RUSTDOCFLAGS="-Dwarnings"`)

### Error handling

- Use `thiserror` for library error types (defined in `gang-core::error`)
- Use `anyhow` in the CLI crate for ad-hoc errors
- Return `Result` instead of panicking
- Include context in error messages: what failed and why

## Project structure

### Adding a new broker

1. Add the `BrokerOperation` variant to `gang-core::broker`
2. Add the `CapabilityGroup` variant to `gang-core::capability`
3. Update the policy engine in `gang-core::policy` to handle the new group
4. Add the WIT interface to `gang-wasm-host/wit/ganglion.wit`
5. Implement the broker in `gang-ros/src/<name>.rs`
6. Register the broker module in `gang-ros/src/lib.rs`
7. Wire the broker into `gang-ros::agent::RobotAgent`
8. Add tests

### Adding a new capability crate

1. Create the crate under `crates/gang-capability-<name>/`
2. Add it to the workspace members in the root `Cargo.toml`
3. Depend on `gang-core` for shared types
4. Write the capability logic as a pure Rust library (no WASM-specific code)
5. Add tests
6. Document it in `docs/CAPABILITY_AUTHOR_GUIDE.md`

### Adding a new CLI command

1. Add the command variant to the `Commands` enum in `gang-cli/src/main.rs`
2. Implement the handler in `gang-cli/src/commands.rs`
3. Wire the command in the `match cli.command` block
4. Update `docs/CLI_REFERENCE.md`

## DCO (Developer Certificate of Origin)

By contributing to this project, you certify that your contribution is your own work (or you have the right to submit it) under the project's Apache-2.0 license.

Sign your commits with:

```
Signed-off-by: Your Name <your.email@example.com>
```

You can do this automatically with `git commit -s`.

## Reporting issues

Open an issue on GitHub with:

- What you expected to happen
- What actually happened
- Steps to reproduce
- Ganglion version (`gang --version`), OS, Rust version (`rustc --version`)

## License

Contributions are licensed under the Apache-2.0 license, the same as the project.
