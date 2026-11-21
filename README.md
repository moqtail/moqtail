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
	<a href="https://docs.moqtail.dev"><img src="https://img.shields.io/badge/Docs-docs.moqtail.dev-1f6feb" alt="Documentation" /></a>
	<br>
	<br>
	IETF-compliant MOQ Transport (MOQT) libraries and relay components.<br>
	Rust and TypeScript tooling for publishers, subscribers, demos and relay deployments.
</p>

# MOQtail

MOQtail is an IETF-compliant MOQT toolkit for building publisher, subscriber and relay applications. The repository includes Rust and TypeScript libraries, reference clients and a relay that you can run locally or pull as a container image from GHCR. The documentation is at [docs.moqtail.dev](https://docs.moqtail.dev).

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
npm install moqtail custom-media-element media-tracks
```

`<moqtail-video>` is a `<video>` that takes a `moqt://` URL:

```html
<script type="module">
  import { defineMOQtailVideo } from 'moqtail/player/component'
  defineMOQtailVideo()
</script>

<moqtail-video src="moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog" autoplay muted playsinline></moqtail-video>
```

Next: the [quickstart](https://docs.moqtail.dev/guide/quickstart) and the [TypeScript library docs](https://docs.moqtail.dev/ts/).

## Rust library

The draft-22 protocol in Rust (wire models, catalogs and the transport handlers the relay is built on). Source: [`libs/moqtail-rs`](libs/moqtail-rs).

```bash
cargo add moqtail
```

Encode a control message and parse it back:

```rust
use moqtail::model::control::{control_message::ControlMessage, goaway::GoAway};

let msg = ControlMessage::Goaway(Box::new(GoAway::new(Some("moqt://relay.example.com".into()), 5000)));
let mut bytes = msg.serialize().unwrap();
assert_eq!(ControlMessage::deserialize(&mut bytes).unwrap(), msg);
```

> [!NOTE]
> The library is under active development and its API may change between releases.

Next: the API reference on [docs.rs](https://docs.rs/moqtail).

## Relay

Forwards MoQ traffic between publishers and subscribers, over WebTransport and raw QUIC on one UDP port. It needs a TLS certificate and key ([how to make them](https://docs.moqtail.dev/relay/certificates)).

```bash
docker run --rm \
	-p 4433:4433/udp \
	-v "$PWD/cert.pem:/certs/cert.pem:ro" \
	-v "$PWD/key.pem:/certs/key.pem:ro" \
	ghcr.io/moqtail/relay:latest
```

Next: [install the relay](https://docs.moqtail.dev/relay/install) from the image or from source, [run your own relay](https://docs.moqtail.dev/relay/tutorial) end to end, and [configure it](https://docs.moqtail.dev/relay/configure).

## Contributing

Contributions are welcome. Open an issue or submit a pull request for improvements, bug fixes, documentation, or interoperability work. [CONTRIBUTING.md](CONTRIBUTING.md) covers the repository layout, building and testing.
