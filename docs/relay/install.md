---
description: Run the MOQtail relay from its container image, or build it from source.
---

# Install the relay

Use the container image to run a released relay. Build from source to run a branch, change the code, or leave out the HTTP API.

Either way, the relay needs a TLS certificate and key in PEM files. To make them, see [Set up TLS certificates](/relay/certificates).

## Run the container image

The image is `ghcr.io/moqtail/relay`, built for `linux/amd64`.

```sh
docker run --rm \
  -p 4433:4433/udp \
  -v "$PWD/cert.pem:/certs/cert.pem:ro" \
  -v "$PWD/key.pem:/certs/key.pem:ro" \
  ghcr.io/moqtail/relay:latest
```

The relay listens on UDP port 4433 for both WebTransport and raw QUIC. It reads the certificate and key from `/certs` inside the container.

::: tip
The image runs as an unprivileged user. If `key.pem` can only be read by you, as `mkcert` makes it, add `--user "$(id -u):$(id -g)"` to `docker run`.
:::

Pick a tag:

| Tag                        | What it is                                    |
| -------------------------- | --------------------------------------------- |
| `latest`                   | The newest release                            |
| `<version>`, e.g. `0.19.0` | A release, from the git tag `relay@<version>` |
| `<branch>`, e.g. `main`    | The latest build of a branch                  |
| `sha-<commit>`             | The build of one commit                       |

### Pass a config file

Mount the file and add the flags after the image name. They replace the image's default arguments, so give the certificate flags again:

```sh
docker run --rm \
  -p 4433:4433/udp \
  -v "$PWD/cert.pem:/certs/cert.pem:ro" \
  -v "$PWD/key.pem:/certs/key.pem:ro" \
  -v "$PWD/relay.yaml:/relay.yaml:ro" \
  ghcr.io/moqtail/relay:latest \
  --config /relay.yaml --cert-file /certs/cert.pem --key-file /certs/key.pem
```

If the config file turns on the HTTP API, publish its TCP port too. [Monitor the relay](/relay/observability) covers the address and access settings it needs.

## Build from source

You need a recent stable [Rust](https://www.rust-lang.org/tools/install) toolchain. The image builds with Rust 1.94.

```sh
git clone https://github.com/moqtail/moqtail.git
cd moqtail
cargo build -p relay --release
```

The binary is `target/release/relay`. Run it with your certificate and key:

```sh
./target/release/relay --cert-file cert.pem --key-file key.pem
```

Check the build with `relay --version`.

### Leave out the HTTP API

The HTTP API, metrics and MCP server are a default Cargo feature. To build the relay without them, for example as a baseline in performance tests:

```sh
cargo build -p relay --release --no-default-features
```

### Build the image yourself

From the repository root:

```sh
docker build -f apps/relay/Dockerfile -t moqtail-relay .
```

## More information

- [Configure the relay](/relay/configure)
- [Relay CLI](/relay/reference/cli)
- [Run your own relay](/relay/tutorial), a tutorial that starts from nothing
