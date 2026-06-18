<p align="center">
	<img src="https://moqtail.dev/moqtail_medal.svg" alt="MOQtail" width="240" />
	<br>
	<br>
	<a href="https://github.com/moqtail/moqtail/actions/workflows/rust.yml">
		<img src="https://github.com/moqtail/moqtail/actions/workflows/rust.yml/badge.svg" alt="Rust Checks" />
	</a>
	<a href="https://github.com/moqtail/moqtail/actions/workflows/js.yml">
		<img src="https://github.com/moqtail/moqtail/actions/workflows/js.yml/badge.svg" alt="JavaScript Checks" />
	</a>
	<a href="https://github.com/moqtail/moqtail/blob/main/LICENSE">
		<img src="https://img.shields.io/badge/license-Apache--2.0-0f172a" alt="License: Apache-2.0" />
	</a>
	<a href="https://github.com/orgs/moqtail/packages/container/package/relay">
		<img src="https://img.shields.io/badge/ghcr-ghcr.io%2Fmoqtail%2Frelay-2496ed?logo=docker&logoColor=white" alt="GHCR Relay Image" />
	</a>
	<a href="https://deepwiki.com/moqtail/moqtail"><img src="https://img.shields.io/badge/DeepWiki-moqtail%2Fmoqtail-blue.svg?logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACwAAAAyCAYAAAAnWDnqAAAAAXNSR0IArs4c6QAAA05JREFUaEPtmUtyEzEQhtWTQyQLHNak2AB7ZnyXZMEjXMGeK/AIi+QuHrMnbChYY7MIh8g01fJoopFb0uhhEqqcbWTp06/uv1saEDv4O3n3dV60RfP947Mm9/SQc0ICFQgzfc4CYZoTPAswgSJCCUJUnAAoRHOAUOcATwbmVLWdGoH//PB8mnKqScAhsD0kYP3j/Yt5LPQe2KvcXmGvRHcDnpxfL2zOYJ1mFwrryWTz0advv1Ut4CJgf5uhDuDj5eUcAUoahrdY/56ebRWeraTjMt/00Sh3UDtjgHtQNHwcRGOC98BJEAEymycmYcWwOprTgcB6VZ5JK5TAJ+fXGLBm3FDAmn6oPPjR4rKCAoJCal2eAiQp2x0vxTPB3ALO2CRkwmDy5WohzBDwSEFKRwPbknEggCPB/imwrycgxX2NzoMCHhPkDwqYMr9tRcP5qNrMZHkVnOjRMWwLCcr8ohBVb1OMjxLwGCvjTikrsBOiA6fNyCrm8V1rP93iVPpwaE+gO0SsWmPiXB+jikdf6SizrT5qKasx5j8ABbHpFTx+vFXp9EnYQmLx02h1QTTrl6eDqxLnGjporxl3NL3agEvXdT0WmEost648sQOYAeJS9Q7bfUVoMGnjo4AZdUMQku50McDcMWcBPvr0SzbTAFDfvJqwLzgxwATnCgnp4wDl6Aa+Ax283gghmj+vj7feE2KBBRMW3FzOpLOADl0Isb5587h/U4gGvkt5v60Z1VLG8BhYjbzRwyQZemwAd6cCR5/XFWLYZRIMpX39AR0tjaGGiGzLVyhse5C9RKC6ai42ppWPKiBagOvaYk8lO7DajerabOZP46Lby5wKjw1HCRx7p9sVMOWGzb/vA1hwiWc6jm3MvQDTogQkiqIhJV0nBQBTU+3okKCFDy9WwferkHjtxib7t3xIUQtHxnIwtx4mpg26/HfwVNVDb4oI9RHmx5WGelRVlrtiw43zboCLaxv46AZeB3IlTkwouebTr1y2NjSpHz68WNFjHvupy3q8TFn3Hos2IAk4Ju5dCo8B3wP7VPr/FGaKiG+T+v+TQqIrOqMTL1VdWV1DdmcbO8KXBz6esmYWYKPwDL5b5FA1a0hwapHiom0r/cKaoqr+27/XcrS5UwSMbQAAAABJRU5ErkJggg==" alt="DeepWiki"></a>
	<br>
	<br>
	Draft 16 MOQ Transport (MOQT) libraries and relay components.<br>
	Rust and TypeScript tooling for publishers, subscribers, demos and relay deployments.
</p>

# MOQtail

MOQtail is a draft 16-compliant MOQT toolkit for building publisher, subscriber, and relay applications. The repository includes Rust and TypeScript libraries, reference clients, and a relay that can be run locally or pulled as a container image from GHCR.

> [!IMPORTANT]
> **To cite MOQtail in your academic research and elsewhere, please use:**
>
> **Zafer Gurel, Deniz Ugur and Ali C. Begen, "MOQtail: open-source, IETF-compliant MOQT protocol libraries," in _Proc. ACM Multimedia Systems Conf. (MMSys)_, Hong Kong, Hong Kong, Apr. 2026 ([DOI: 10.1145/3793853.3799817](https://doi.org/10.1145/3793853.3799817))**

## Components

### moqtail-ts

The TypeScript library targets browser and WebTransport-based MoQ applications.

Highlights:

- Type-safe application APIs
- WebTransport integration
- Client-side development workflow with the demo app

Library documentation: [libs/moqtail-ts/README.md](libs/moqtail-ts/README.md)

### moqtail-rs

The Rust library provides the core protocol implementation and utilities used by the relay and other Rust applications in this workspace.

Library documentation: [libs/moqtail-rs/README.md](libs/moqtail-rs/README.md)

### Relay

The relay is the deployable Rust service that forwards MoQ messages between publishers and subscribers.

Local run:

```bash
cargo run -p relay -- --port 4433 --cert-file apps/relay/cert/cert.pem --key-file apps/relay/cert/key.pem
```

Container image:

```bash
docker run --rm \
	-p 4433:4433/udp \
	-v "$PWD/apps/relay/cert/cert.pem:/certs/cert.pem:ro" \
	-v "$PWD/apps/relay/cert/key.pem:/certs/key.pem:ro" \
	ghcr.io/moqtail/relay:latest
```

Release images are published to `ghcr.io/moqtail/relay` with `latest` and version tags from `relay@*` releases. Branch and commit SHA tags are also published for CI builds.

To build the image locally from the workspace root:

```bash
docker build -f apps/relay/Dockerfile -t moqtail-relay .
```

For local certificate generation and browser trust setup, see [apps/relay/cert/README.md](apps/relay/cert/README.md).

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [Node.js](https://nodejs.org/) 18 or newer
- [npm](https://www.npmjs.com/)
- [Docker](https://www.docker.com/) for containerized relay builds and runs

### Installation

```bash
git clone https://github.com/moqtail/moqtail.git
cd moqtail
npm install
```

## Contributing

Contributions are welcome. Open an issue or submit a pull request for improvements, bug fixes, documentation, or interoperability work.
