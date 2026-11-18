<p align="center">
	<img src="https://moqtail.dev/moqtail_medal.svg" alt="MOQtail" width="240" />
	<br>
	<br>
	<a href="https://datatracker.ietf.org/doc/draft-ietf-moq-transport/22/"><img src="https://img.shields.io/badge/MOQT-draft--22-0f766e" alt="MOQT draft-22" /></a>
	<a href="https://github.com/moqtail/moqtail/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-0f172a" alt="License: Apache-2.0" /></a>
	<a href="https://doi.org/10.1145/3793853.3799817"><img src="https://img.shields.io/badge/DOI-10.1145%2F3793853.3799817-b31b1b" alt="DOI" /></a>
	<br>
	<a href="https://www.npmjs.com/package/moqtail"><img src="https://img.shields.io/npm/v/moqtail?logo=npm" alt="npm" /></a>
	<a href="https://crates.io/crates/moqtail"><img src="https://img.shields.io/crates/v/moqtail?logo=rust" alt="crates.io" /></a>
	<a href="https://github.com/orgs/moqtail/packages/container/package/relay"><img src="https://img.shields.io/badge/ghcr-ghcr.io%2Fmoqtail%2Frelay-2496ed?logo=docker&logoColor=white" alt="GHCR Relay Image" /></a>
	<a href="https://deepwiki.com/moqtail/moqtail"><img src="https://img.shields.io/badge/Ask-DeepWiki-1f6feb" alt="Ask DeepWiki" /></a>
	<br>
	<br>
	IETF-compliant MOQ Transport (MOQT) libraries and relay components.<br>
	Rust and TypeScript tooling for publishers, subscribers, demos and relay deployments.
</p>

# MOQtail

MOQtail is an IETF-compliant MOQT toolkit for building publisher, subscriber and relay applications. The repository includes Rust and TypeScript libraries, reference clients and a relay that can be run locally or pulled as a container image from GHCR. The relay and the Rust test client support both WebTransport and raw QUIC, and the relay serves both on the same port.

> [!WARNING]
> **Branch status:** `main` is tracking draft-22, we continue to work on it until the next draft is released. For previous drafts, you can use the following branches:
>
> - [`draft-14`](https://github.com/moqtail/moqtail/tree/draft-14) branch.
> - [`draft-16`](https://github.com/moqtail/moqtail/tree/draft-16) branch.
> - [`draft-18`](https://github.com/moqtail/moqtail/tree/draft-18) branch.

> [!IMPORTANT]
> **To cite MOQtail in your academic research and elsewhere, please use:**
>
> **Zafer Gurel, Deniz Ugur and Ali C. Begen, "MOQtail: open-source, IETF-compliant MOQT protocol libraries," in _Proc. ACM Multimedia Systems Conf. (MMSys)_, Hong Kong, Hong Kong, Apr. 2026 ([DOI: 10.1145/3793853.3799817](https://doi.org/10.1145/3793853.3799817))**

## Packages

| Package                                                                                     | Install                             | What it is                                                                                     |
| ------------------------------------------------------------------------------------------- | ----------------------------------- | ---------------------------------------------------------------------------------------------- |
| [`moqtail`](https://www.npmjs.com/package/moqtail) (npm)                                    | [Browser library](#browser-library) | MOQT client and MSF/CMSF player for the browser, with the `<moqtail-video>` web component      |
| [`moqtail`](https://crates.io/crates/moqtail) (crates.io)                                   | [Rust library](#rust-library)       | The MOQT protocol in Rust: control messages, data streams, parameters, properties and catalogs |
| [`ghcr.io/moqtail/relay`](https://github.com/orgs/moqtail/packages/container/package/relay) | [Relay](#relay)                     | The deployable relay that forwards MoQ traffic between publishers and subscribers              |

## Browser library

Runs in Chrome and Safari 26.4+ (WebTransport). Source: [`libs/moqtail-ts`](libs/moqtail-ts).

```bash
npm install moqtail
```

The `<moqtail-video>` web component needs two optional peer dependencies:

```bash
npm install custom-media-element media-tracks
```

**`<moqtail-video>`**: a `<video>` that takes a `moqt://` URL and installs the default buffer and ABR plugins.

```html
<script type="module">
  import { defineMOQtailVideo } from 'moqtail/player/component'
  defineMOQtailVideo()
</script>

<moqtail-video src="moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog" autoplay muted playsinline></moqtail-video>
```

**Player API**: build the chain yourself and pick the plugins.

```ts
import { MOQtailPlayer, liveBuffer } from 'moqtail/player'

const player = new MOQtailPlayer({ plugins: [liveBuffer()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog')
  .sink(video)

await player.play()
```

The client on its own (subscribe, fetch, publish), events, elements and plugins are in [libs/moqtail-ts/README.md](libs/moqtail-ts/README.md).

## Rust library

Published on crates.io as `moqtail`. It provides the draft-22 protocol (wire models for control messages, data streams, parameters, properties and MSF/CMSF catalogs) and the transport handlers the relay is built on. Source: [`libs/moqtail-rs`](libs/moqtail-rs).

```bash
cargo add moqtail
```

> [!NOTE]
> The library is under active development and its API may change between releases.

## Relay

The relay accepts WebTransport and raw QUIC connections on the same UDP port and tells them apart by ALPN. It needs a TLS certificate and key; for local development, [apps/relay/cert/README.md](apps/relay/cert/README.md) shows how to make them with mkcert and how to make browsers trust them.

### With Docker

```bash
docker run --rm \
	-p 4433:4433/udp \
	-v "$PWD/cert.pem:/certs/cert.pem:ro" \
	-v "$PWD/key.pem:/certs/key.pem:ro" \
	ghcr.io/moqtail/relay:latest
```

To pass a config file, mount it and add the flags after the image name (they replace the default arguments, so keep the certificate flags):

```bash
docker run --rm \
	-p 4433:4433/udp \
	-v "$PWD/cert.pem:/certs/cert.pem:ro" \
	-v "$PWD/key.pem:/certs/key.pem:ro" \
	-v "$PWD/relay.yaml:/relay.yaml:ro" \
	ghcr.io/moqtail/relay:latest --config /relay.yaml --cert-file /certs/cert.pem --key-file /certs/key.pem
```

Release images are tagged `latest` and with the version of each `relay@*` release. Branch and commit SHA tags are also published for CI builds.

### From source

Requires [Rust](https://www.rust-lang.org/tools/install).

```bash
git clone https://github.com/moqtail/moqtail.git
cd moqtail
cargo run -p relay --release -- --cert-file apps/relay/cert/cert.pem --key-file apps/relay/cert/key.pem
```

To build the image yourself, run `docker build -f apps/relay/Dockerfile -t moqtail-relay .` from the repository root.

### Configuration

Every setting lives in one YAML file passed with `--config`; `--bind`, `--cert-file` and `--key-file` override it. [apps/relay/relay.example.yaml](apps/relay/relay.example.yaml) lists every key with its default, including the cache, load limits, SSTS, relay mesh (`hop_id`, `peers`) and the HTTP API.

### Transports: WebTransport and raw QUIC

Clients address the relay with a `moqt://` URL either way; the transport is chosen separately.

- **WebTransport** (ALPN `h3`): used by browsers and `moqtail` on npm. The `moqt://` URL becomes an `https://` URL for the HTTP CONNECT request, which carries the authority and path.
- **Raw QUIC**: raw QUIC has no HTTP CONNECT, so the client sends the authority and path as CLIENT_SETUP parameters instead.

The Rust test client in [`apps/client`](apps/client) speaks both and picks one with `--transport` (`web-transport` by default, or `quic`):

```bash
cargo run --bin client -- -c publish --server moqt://127.0.0.1:4433
cargo run --bin client -- -c publish --server moqt://127.0.0.1:4433 --transport quic
```

Its commands (publish, publish-namespace, subscribe, subscribe-namespace, subscribe-tracks, fetch) and options are in [apps/client/README.md](apps/client/README.md).

## Contributing

Contributions are welcome. Open an issue or submit a pull request for improvements, bug fixes, documentation, or interoperability work. [CONTRIBUTING.md](CONTRIBUTING.md) covers the repository layout, building and testing.
