# TEST KEYS — DO NOT USE

These are **deterministic test identity keys** for the Docker test harness
only. They are committed to the repository and provide **zero security**.
Never use them for a real relay, robot, or operator.

Each `<scenario>-relay.key` file is a raw 32-byte Ed25519 secret key
(the format `gang_core::identity::Keypair::load` expects), generated once
with `python3 -c "import secrets; ...token_bytes(32)"`. Each scenario's
relay container mounts its key read-only and points `GANG_KEY_PATH` at it,
so the relay's peer ID is stable across runs of that scenario.

The corresponding peer IDs are not precomputed here (deriving them requires
the `gang` binary / Blake3 of the public key). Instead, on startup the relay
entrypoint wrapper (`../scripts/relay-entrypoint.sh`) resolves its own peer
ID via `gang identity show` and publishes its dialable multiaddr to a shared
volume; agents wait for that file and dial it (see
`../scripts/agent-entrypoint.sh`).
