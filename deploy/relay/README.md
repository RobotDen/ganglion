# Ganglion Bootstrap Relay

A Ganglion node running in relay-server mode. It provides circuit relay v2
services so that robot agents behind NAT can accept inbound connections from
operators.

## What the relay does

- Accepts incoming libp2p connections on TCP 4001 and QUIC (UDP 4001).
- Provides relay reservations to any peer that requests one (no registration).
- Forwards relayed traffic between operator and robot until DCUtR upgrades the
  connection to a direct path.
- Does **not** inspect, store, or log relayed application data.

## Deploying

### Prerequisites

- Docker and Docker Compose
- A VPS with a public IPv4 address and ports 4001 TCP+UDP open

### Start the relay

```bash
cd deploy/relay
docker compose up -d
```

The first run builds the container image (takes a few minutes for the Rust
compile) and generates an Ed25519 identity key. The container sets
`GANG_KEY_PATH=/data/identity.key`, so the key is created inside the
`relay-data` volume rather than the container filesystem.

### Get the relay's peer ID

```bash
docker compose exec relay gang identity show
```

Copy the peer ID. You will use it to configure robots and operators.

### Stop the relay

```bash
docker compose down
```

Because `GANG_KEY_PATH` points at `/data/identity.key`, the identity key
persists in the `relay-data` volume. Restarting reuses the same peer ID as
long as the volume is kept.

## Configuring robots to use the relay

Add the relay's multiaddr to the robot agent's config file:

```toml
relay_addrs = [
  "/dns4/relay.gang.tafy.dev/tcp/4001/p2p/<PEER_ID>"
]
```

Replace `<PEER_ID>` with the peer ID from `gang identity show` on the relay.

## Running locally (without Docker)

```bash
cargo run -p gang -- relay
# or with a custom port:
cargo run -p gang -- relay --port 9001
```

## Resource usage

The relay is lightweight. Expected resource consumption on a small VPS:

- **CPU**: negligible (< 1% idle, spikes during relay negotiation)
- **Memory**: ~20-40 MB RSS
- **Bandwidth**: proportional to relayed traffic; most connections upgrade to
  direct via DCUtR within seconds
- **Cost**: $5-20/month on a small VPS (1 vCPU, 512 MB-1 GB RAM)

## Rate limits

The relay uses libp2p's default circuit relay v2 limits:

- Max 128 concurrent relay reservations
- Max 64 KB/s per relayed connection
- Reservation duration: 1 hour
- Max data transfer per relay circuit: 128 KiB

These defaults are sufficient for the relay's role as a rendezvous point.
Most connections upgrade to direct via DCUtR within seconds.

## Security

- The relay does not require authentication to reserve a slot. Any peer can
  request a relay reservation.
- All traffic is encrypted end-to-end with Noise. The relay cannot read
  relayed application data.
- The relay's identity key is stored in the Docker volume at `/data/identity.key`
  (the container sets `GANG_KEY_PATH` to that path). Back it up if you want to
  preserve the peer ID across volume recreation.
