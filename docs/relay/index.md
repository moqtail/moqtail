---
description: What the MOQtail relay does, what it offers, and the ports and versions it uses.
---

# Relay

The MOQtail relay sits between publishers and subscribers. A publisher sends a track to the relay once, and the relay forwards it to every subscriber that asks for it. Publishers and subscribers never connect to each other, so a publisher's upload stays the same however many subscribers there are.

## What it does

**WebTransport and raw QUIC on one port.** The relay accepts both on the same UDP port and tells them apart by the ALPN the client asks for. Browsers connect over WebTransport; native clients and other relays use raw QUIC. Objects travel on streams or as datagrams on either. [Transports](/guide/concepts/transports) explains how this works.

**Fan-out and cache.** Each track reaches the relay once, and the relay copies its objects to every subscriber. It keeps the recent groups of each track in a cache, answers FETCH requests from it, and asks the publisher only for what is missing. A subscriber that falls behind skips the rest of a group, so it cannot grow the relay's memory without bound. [How the relay forwards](/guide/concepts/relay) goes into the details.

**Relay mesh.** Relays connect to each other over raw QUIC and tell each other which namespaces their clients publish. A relay routes each request to the cheapest source it knows for the namespace, and Hop IDs keep announcements from going around in a loop. A subscriber on one relay can then receive a track published on another. See [Build a relay mesh](/relay/clustering).

**Observability.** An optional HTTP API under `/api/v1` reports connections, tracks, the cache and the time objects spend in the relay. It also serves Prometheus metrics, with a Grafana dashboard in `apps/relay-api/grafana`, streams events, and runs an MCP server so an AI agent can inspect the relay. Separately, qlog traces record the control messages of every connection. The API is a Cargo feature: a relay built without it carries none of this code. See [Monitor the relay](/relay/observability) and [Connect an AI agent](/relay/mcp).

**Post-quantum TLS.** Connections use TLS 1.3 only. Next to the classical key exchanges, the relay offers X25519MLKEM768, a hybrid post-quantum key exchange that some QUIC clients use by default.

**Sender-side track switching (SSTS).** With SSTS, the relay serves one logical track out of a switching set: several encodings of the same content. For each group it forwards one of them, picked from the bandwidth to that subscriber by an algorithm the relay runs. SSTS is built on a provisional draft (`draft-wilaw-moq-moqt-ssts`), so it is off by default, and a relay that leaves it off never puts its options on the wire. `ssts_enable` turns it on.

**Conformance tested.** The wire constants of the Rust library the relay is built on (message types, parameters, error codes and varint encodings) are tested against JSON fixtures transcribed from the draft, in [`dev/conformance/draft22`](https://github.com/moqtail/moqtail/tree/main/dev/conformance/draft22). The `moqtail` TypeScript package is tested against the same files, so the two use the same codepoints.

## Ports and versions

| What          | Value                                                                |
| ------------- | -------------------------------------------------------------------- |
| Relay version | 0.19.0                                                               |
| MOQT          | draft-ietf-moq-transport-22, `moqt-22`                               |
| ALPN          | `h3` for WebTransport, `moqt-22` for raw QUIC                        |
| Relay port    | UDP 4433, on every IPv4 and IPv6 address (`[::]:4433`)               |
| HTTP API      | TCP `127.0.0.1:4480`, when `api.enable` is on                        |
| Container     | `ghcr.io/moqtail/relay`, for `linux/amd64`                           |
| Older drafts  | The `draft-14`, `draft-16` and `draft-18` branches of the repository |

A relay speaks one MOQT draft and refuses a client that offers only other versions.

## Where to go next

- [Run your own relay](/relay/tutorial), a tutorial that goes from a certificate to objects passing through your relay.
- To set a relay up: [Install the relay](/relay/install), [Set up TLS certificates](/relay/certificates) and [Configure the relay](/relay/configure).
- To run it: [Build a relay mesh](/relay/clustering), [Monitor the relay](/relay/observability), [Connect an AI agent](/relay/mcp) and [Run the relay in production](/relay/production).
- To look things up: [Relay CLI](/relay/reference/cli), [Configuration file](/relay/reference/config) and [HTTP API](/relay/reference/http-api).
