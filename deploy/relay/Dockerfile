FROM rust:1.85-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release -p gang

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/gang /usr/local/bin/gang
EXPOSE 4001/tcp 4001/udp
VOLUME ["/data"]
ENV GANG_KEY_PATH=/data/identity.key
ENTRYPOINT ["gang"]
CMD ["relay", "--listen-addr", "/ip4/0.0.0.0/tcp/4001", "--listen-addr", "/ip4/0.0.0.0/udp/4001/quic-v1"]
