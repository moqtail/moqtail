# syntax=docker/dockerfile:1.7

FROM rust:1.94-bookworm AS builder

WORKDIR /workspace

COPY Cargo.toml ./
COPY Cargo.lock ./
COPY apps ./apps
COPY libs ./libs
COPY .git ./.git

RUN cargo build -p relay --release \
    && mkdir -p /workspace/rootfs/certs /workspace/rootfs/tmp

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

COPY --from=builder /workspace/target/release/relay /usr/local/bin/relay
COPY --from=builder --chown=nonroot:nonroot /workspace/rootfs/ /

EXPOSE 4433/udp

ENTRYPOINT ["/usr/local/bin/relay"]
CMD ["--host", "0.0.0.0", "--cert-file", "/certs/cert.pem", "--key-file", "/certs/key.pem"]
