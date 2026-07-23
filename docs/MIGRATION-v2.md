# Migrating to Ganglion v2.0.0

Ganglion 2.0.0 is a security- and quality-hardening release. It contains
breaking changes to the wire protocol, on-disk trust configuration, and several
public library APIs. This guide lists exactly what an operator, a robot
maintainer, and a library consumer must do to upgrade.

Read the [CHANGELOG](../CHANGELOG.md) `[2.0.0]` section for the full list of
changes; this document is the actionable subset.

## TL;DR

1. **Upgrade every node at once.** Control requests now carry a replay
   nonce+timestamp; a pre-2.0 peer talking to a 2.0 agent (or vice versa) is
   rejected. Do not run a mixed fleet.
2. **Regenerate trust configuration.** Peer-id derivation changed (SEC-03); any
   recorded *remote* peer ids and `peer_rules` keyed on the old id must be
   recreated from each peer's current id.
3. **Re-sign and re-publish capabilities through signed manifests.** The
   registry now refuses unsigned entries.
4. **Fix any malformed policy/trust files.** Startup now fails closed instead of
   falling back to permissive.
5. **Library consumers:** update to the new `Cid::parse` / `Registry::publish` /
   `PeerId` APIs, add wildcard match arms for `#[non_exhaustive]` wire enums,
   and enable any tokio features you were getting transitively.

## 1. Upgrade all agents and operators together (replay nonce)

Control requests (`gang deploy`/`run`/`caps` and the agent serve path) now
include a per-request nonce and timestamp, and the agent rejects stale or
replayed requests. This is an additive field, but a 2.0 agent will reject a
request that arrives without a valid nonce, and a pre-2.0 agent will ignore the
new field and fail verification differently.

**Action:** upgrade the relay, every robot agent, and every operator CLI to
2.0.0 in the same maintenance window. There is no supported mixed-version mode.

## 2. Regenerate trust configuration (SEC-03 peer-id unification)

Before 2.0, the libp2p transport identified a *remote* peer using a
libp2p-multihash-based id that did **not** match the id `gang-core` derives from
the peer's Ed25519 key. As a result, trust-store `peer_rules` keyed on a remote
id were never actually enforced. 2.0 derives the remote id from the peer's raw
Ed25519 public key using the same canonical scheme everywhere, so `peer_rules`
now work — but any id you previously recorded for a *remote* peer is likely
wrong.

A robot's **own** id (derived from its own key) is unchanged. Only recorded
*remote* ids are affected.

**Action on each operator machine:**

1. Re-read each robot's current id from the robot itself:
   ```bash
   gang identity show          # run on the robot (or read it from `gang agent` startup output)
   ```
2. Re-register the peer with the correct id:
   ```bash
   gang peer remove <name>
   gang peer add <name> <current-peer-id> --relay <relay-multiaddr>
   ```
3. Recreate any policy `peer_rules` that referenced the old id, using the
   current id. If a rule stops matching after upgrade, this is why.
4. Clear any stale stored host key so TOFU re-pins the correct one:
   ```bash
   gang peer trust-reset <name>
   ```

## 3. Re-sign and re-publish capabilities (SEC-15 + `--capabilities`)

Two related changes:

- `gang sign` no longer auto-extracts declared capabilities from the component.
  Pass them explicitly with `--capabilities`; if you omit the flag, signing
  falls back to a permissive default set and prints a warning (almost never what
  you want).
- `gang registry publish` now **requires** an adjacent signed manifest and
  authenticates the entry against it. `Registry::publish` takes a
  `&SignedManifest`. Unsigned publishing is no longer possible.

**Action:**

```bash
# Re-sign with explicit capabilities (example)
gang sign my-tool.component.wasm \
    --name my-tool --component-version 0.2.0 \
    --capabilities diagnostics,logs

# Publish the signed component (manifest is picked up automatically)
gang registry publish my-tool.component.wasm \
    --description "..." --tags diagnostics
```

`--component-version` has the alias `--version`; both set the *component's*
semantic version and are distinct from the CLI's own `-V`.

## 4. Fix malformed policy and trust files (fail-closed)

A malformed or unreadable policy file or trust store now **aborts agent
startup** instead of silently falling back to a permissive policy. If an agent
that used to start now refuses to, check its policy and trust-store files first.

Also note:

- Identity key files with permissions looser than `0600` are repaired to `0600`
  (with a warning) on load.
- The audit log is now a Blake3 hash chain with `0600` permissions and a
  `verify_chain()` integrity check. Existing legacy audit logs remain readable;
  only newly appended records are chained.

**Action:** validate policy/trust files before rolling out
(`gang config ...` / a dry-run agent start), and ensure key and audit files
have the expected owner and permissions.

## 5. Library consumers (`gang-core` and friends)

If you depend on the Ganglion crates directly:

- **`Cid::parse`** — content ids are now parsed with a fallible
  `Cid::parse(&str) -> Result<Cid, CidError>`. Replace loose string handling.
- **`Registry::publish`** — the signature is now
  `publish(&mut self, entry: RegistryEntry, signed_manifest: &SignedManifest)`
  and verifies the manifest. Provide a signed manifest.
- **Strict `PeerId` validation** — constructing/parsing a `PeerId` from a string
  now validates the `12D3-` prefix and length and returns an error on malformed
  input. Handle the `Result`.
- **`#[non_exhaustive]` wire enums** — `ControlMessage`, `InvokeStatus`, and
  `BrokerOperation` are now `#[non_exhaustive]`. Add a wildcard (`_ =>`) arm to
  any exhaustive `match` over them.
- **Reduced tokio feature set (CODE-15)** — the library crates now enable only a
  minimal tokio feature set; the `gang` binary widens it to `full`. If your code
  relied on a tokio feature that was previously enabled transitively through a
  Ganglion crate, enable it explicitly in your own `Cargo.toml`.

## Not yet available in 2.0

- **Relay-mediated remote dispatch** (`gang deploy`/`run`/`caps` to a remote
  robot) is still WIP. A resolved remote target exits with a "not yet
  implemented (ADR-020 Phase 32)" message; the local fallback path works. Plan
  around local execution until Phase 32 lands.
- `gang logs`, `gang list`, `gang connect` remain `[WIP]` and exit non-zero.
- `gang transport-stats` prints simulated data (clearly labeled) until live
  connections land.
