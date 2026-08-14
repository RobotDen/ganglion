# Packaging

## Homebrew (macOS + Linux)

```bash
brew install robotden/tap/gang
```

The tap lives at [RobotDen/homebrew-tap](https://github.com/RobotDen/homebrew-tap);
after each release, run its `scripts/update-formula.sh <version>` to point the
formula at the new tarballs.

## cargo-binstall

`cargo binstall gang` resolves the prebuilt release tarballs via
`[package.metadata.binstall]` in `crates/gang-cli/Cargo.toml` — no compile.

## Debian / Ubuntu (.deb)

`[package.metadata.deb]` in `crates/gang-cli/Cargo.toml` defines the package;
the release workflow builds `gang_<version>_{amd64,arm64}.deb` natively on
each Linux runner. To wire it into `.github/workflows/release.yml` (requires
a committer token with workflow scope), make these three edits:

1. After the "Build gang binary" step, add:

```yaml
      - name: Build .deb (Linux)
        if: contains(matrix.target, 'linux')
        run: |
          cargo install cargo-deb --locked
          version="${GITHUB_REF_NAME#v}"
          mkdir -p dist
          cargo deb -p gang --target ${{ matrix.target }} --no-build \
            -o "dist/gang_${version}_$(dpkg --print-architecture).deb"
```

2. In the checksum step, widen the glob so debs are covered:

```yaml
          sha256sum gang-*.tar.gz gang_*.deb > SHA256SUMS
```

3. In the release-upload `files:` list, add:

```yaml
            dist/gang_*.deb
```

Local build for testing: `cargo build --release -p gang && cargo deb -p gang --no-build`.
