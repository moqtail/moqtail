# syntax=docker/dockerfile:1.7

FROM rust:1.94-bookworm AS chef

RUN cargo install cargo-chef --locked --version 0.1.78

WORKDIR /workspace

FROM chef AS planner

COPY Cargo.toml ./
COPY Cargo.lock ./
COPY apps ./apps
COPY libs ./libs
COPY vendor ./vendor

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

# Downloads and compiles dependencies only; this layer stays cached until a manifest or Cargo.lock changes.
COPY --from=planner /workspace/recipe.json ./
# The [patch.crates-io] wtransport is a path dependency cook must compile.
COPY vendor ./vendor
RUN cargo chef cook -p relay --release --recipe-path recipe.json

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
CMD ["--cert-file", "/certs/cert.pem", "--key-file", "/certs/key.pem"]
