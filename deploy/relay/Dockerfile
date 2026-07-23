# TODO: pin by digest in CI where docker is available
# (e.g. docker buildx imagetools inspect rust:1.85-slim-bookworm)
FROM rust:1.85-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release -p gang

# TODO: pin by digest in CI where docker is available
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates netcat-openbsd && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/gang /usr/local/bin/gang

# Run as a dedicated non-root user; /data holds the relay identity key.
RUN useradd --system --uid 10001 --create-home --home-dir /home/gang gang \
    && mkdir -p /data \
    && chown gang:gang /data
USER gang

EXPOSE 4001/tcp 4001/udp
VOLUME ["/data"]
# Identity persists in the /data volume (see deploy/relay/README.md).
ENV GANG_KEY_PATH=/data/identity.key
ENTRYPOINT ["gang"]
CMD ["relay", "--listen-addr", "/ip4/0.0.0.0/tcp/4001", "--listen-addr", "/ip4/0.0.0.0/udp/4001/quic-v1"]
