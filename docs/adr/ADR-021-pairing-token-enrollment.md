# ADR-021: Pairing-token enrollment (`gang pair` / `gang join`)

**Status:** Accepted; implemented
**Date:** 2026-08-06

> **Implementation status.** Landed. `gang pair` (operator) mints a short-lived,
> single-use pairing token and prints one robot-side line; `gang join <token>`
> (robot) dials out, reserves a circuit, enrolls, and stays online as the agent.
> The operator records the robot under the identity libp2p authenticated on the
> wire and it appears in `gang peer list`, after which `gang deploy`/`run`/`caps`
> work. Token mint/verify/single-use logic lives in `gang-core::pairing`;
> integration coverage is in `crates/gang-cli/tests/pairing.rs`. QR output is
> deferred (no terminal-QR crate in the workspace dependency table — see below).

## Context

Registering a robot previously required the operator to copy the robot's
dialable libp2p id (printed by `gang agent`) and hand-run `gang peer add <name>
<libp2p-id> --relay <multiaddr>`. That is two error-prone copy-paste steps across
two machines and exposes internal identifiers to the operator. Issue #5 asks for
"the Tailscale move": the operator runs one command, gets a single copy-paste
line for the robot, and the robot appears in the peer list — no manual id copying
in either direction.

The connectivity substrate to do this already exists (ADR-020): a circuit-relay
v2 server, outbound-only transports that reserve circuits, the
`/ganglion/control/1.0` request/response protocol, TOFU host-key machinery, and
— critically — `GanglionStream::remote_peer`, the gang id derived in
`handle_rpc_message` from the **Ed25519 key libp2p's Noise handshake
authenticated** on the stream. What was missing was an enrollment exchange and a
credential proving a robot was invited.

## Decision

Adopt a **pairing token**: a bearer credential the operator mints and the robot
presents over an authenticated circuit. Chosen shape is **design (A)** from the
issue — full auto-registration over the relay, giving genuine Tailscale-parity —
built entirely by composing existing machinery rather than inventing new crypto
or a new service.

### Token format

Versioned, URL-safe, self-describing:

```
gang1_<base64url(CBOR({ version, relay_addr, operator_libp2p_id, secret[32], expires_at_ms }))>
```

- `version` (`1`): unknown versions are rejected, not mis-parsed.
- `relay_addr`: the dialable relay multiaddr the robot should dial.
- `operator_libp2p_id`: the operator's dialable base58 id. The robot dials *this*
  through the relay circuit; Noise authenticates the operator end-to-end, so the
  robot cryptographically confirms it reached the operator the token names.
- `secret`: 32 bytes of CSPRNG output — the bearer proof of invitation.
- `expires_at_ms`: absolute expiry.

Encoding is base64url without padding (RFC 4648 §5), implemented locally in
`gang-core::pairing` with round-trip/edge-case unit tests, because no `base64`
crate is in the workspace dependency table and the token is not itself secret
material. The type, mint/decode/verify functions, and the operator-side
`authorize_enrollment` decision are pure and unit-tested in `gang-core`.

### Enrollment exchange (design A)

1. `gang pair` loads the operator identity, builds an outbound transport that
   **reserves a circuit** on the relay (so the robot can dial the operator), mints
   the token, prints `gang join gang1_…`, registers an enrollment handler on
   `/ganglion/control/1.0`, and blocks.
2. `gang join <token>` decodes the token, loads/generates the robot identity,
   adds the operator to its trust store (so later deploys are authorized, SEC-03),
   reserves its own circuit, and dials the operator through the relay.
3. The robot sends a `ControlMessage::Enroll { token_secret, name, libp2p_id }`.
4. The operator's handler reads `stream.remote_peer` — the wire-authenticated
   gang id — and calls `authorize_enrollment`, which requires (a) the presented
   secret to match the token in constant time and be unexpired, and (b) the gang
   id **derived from the robot's self-reported `libp2p_id`** to equal the
   wire-authenticated id. It then consumes the token (single-use, one-shot flag),
   records the robot in the peer registry and pre-provisions its host key in the
   operator trust store — all keyed on the wire-authenticated identity — and
   replies `Enrolled { operator_id, robot_id, name }`.
5. The robot verifies the acknowledging `operator_id` matches the token
   (defence-in-depth; libp2p already enforced it on the wire) and, unless
   `--once`, stays online serving the control protocol so the operator can deploy.

### Why the recorded identity is honest

The operator never stores an unauthenticated claim. The gang id comes from
`stream.remote_peer`, derived from the key libp2p proved on the connection. The
robot's self-reported dialable `libp2p_id` is accepted **only** after its embedded
key is shown to derive back to that same wire-authenticated id (a Blake3
truncation collision would be required to defeat this — infeasible). So a robot
cannot be recorded as an identity whose Ed25519 key it does not hold.

## Trust argument

- **Forge:** the secret is 32 CSPRNG bytes; an attacker who never saw a
  minted token cannot produce a secret the operator accepts, so cannot enroll
  into the operator's fleet.
- **Tamper:** flipping bits in the encoded token breaks base64url/CBOR decoding
  or changes the secret; both are rejected.
- **Replay / reuse:** the operator consumes the token on first successful
  enrollment (one-shot flag); a second presentation is rejected. A rejected
  enrollment does *not* consume the token, so a genuine retry still works.
- **Expiry:** `authorize_enrollment` (via `PairingToken::verify`) rejects a token
  past `expires_at_ms`; `gang join` also pre-checks expiry before any network I/O.
- **Wrong id:** the recorded identity is the wire-authenticated one; a claimed
  dialable id that does not derive to it is rejected (`IdentityMismatch`).
- **Wrong operator:** the token names the operator's dialable id; the robot dials
  exactly that id through the circuit, so Noise fails the connection unless it
  reaches the operator the token names.

The token is intentionally **not** operator-signed: the bearer secret is the
credential (like a Tailscale auth key), and the operator holds the canonical copy
in memory for one `gang pair` session to compare and consume. Signing would add
key management without strengthening any of the properties above.

## Consequences

- Enrollment is one line on the robot; the manual `gang peer add` remains as a
  documented fallback for air-gapped or scripted registration.
- `gang pair` is a foreground, blocking command for the duration of one
  enrollment, mirroring `gang up`'s "serve until done" model. Single-use is
  enforced per-session in memory; there is no persistent used-token ledger,
  because each `gang pair` mints a fresh secret and holds it only while waiting.
- **QR output** (issue: "or QR") is deferred: no terminal-QR crate is in the
  workspace dependency table, and this change does not add dependencies without
  approval. `--qr` currently prints an honest note and the copy-paste line;
  wiring a QR renderer is a follow-up once a crate is approved.
- Enrollment rides the existing control protocol and TOFU/replay machinery; no
  parallel crypto was introduced.
