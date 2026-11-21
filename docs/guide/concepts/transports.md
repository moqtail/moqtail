---
description: Why MOQtail speaks MOQT over both WebTransport and raw QUIC, and how one relay port serves both.
---

# Transports

MOQT runs on QUIC. It needs what QUIC offers: many independent streams, so a lost packet only delays the objects on its own stream, and unreliable datagrams for objects that are worthless when late. MOQtail carries MOQT over QUIC in two ways.

## Two ways onto QUIC

**WebTransport** runs over HTTP/3. A browser cannot open a plain QUIC connection, but it can open a WebTransport session, and the session gives it streams and datagrams. This is how the `moqtail` package connects: it uses the browser's `WebTransport` API and offers `moqt-22` as the session's protocol.

**Raw QUIC** puts MOQT straight on a QUIC connection, with `moqt-22` as the TLS ALPN. Native applications use it: the Rust CLI client can, and relays use it to talk to each other in a [relay mesh](/relay/clustering). There is no HTTP layer to set up.

On top of either one, MOQT behaves the same: the same control messages, the same objects, the same priorities.

## One port for both

The relay listens on one UDP port, 4433 by default, and accepts both. It tells them apart during the TLS handshake by the ALPN the client asks for:

- `h3` means HTTP/3, so the connection becomes a WebTransport session.
- A MOQT version such as `moqt-22` means raw QUIC.

So one address serves browsers and native clients alike, and a firewall needs one open UDP port. When the relay starts, it prints both forms of its address:

```
moqtail/0.19.0+<commit> is running on 1 UDP socket(s) -- WebTransport clients: https://localhost:4433  Raw QUIC clients: moqt://localhost:4433
```

Both use TLS 1.3. The relay also offers the post-quantum key exchange X25519MLKEM768, which some QUIC clients use by default.

## `moqt://` and `https://`

A `moqt://` URL names a MOQT endpoint without fixing the transport. Each client maps it to a transport it can use:

- The `moqtail` package turns `moqt://host:port/path` into `https://host:port/path` and opens a WebTransport session there. Without a port it uses 443. An `https://` URL is used as it is.
- Over WebTransport, the authority and path travel in the HTTP CONNECT request. Raw QUIC has no HTTP layer, so a client sends them as parameters of CLIENT_SETUP instead.
- The Rust CLI client takes the transport as an option, `--transport web-transport` or `--transport quic`.
- A relay dials its mesh peers over raw QUIC.

[MSF URLs](/ts/reference/msf-url), which the player takes, are `moqt://` URLs with a fragment that names the track to play.

## Certificates

Both transports need a certificate the client trusts. Browsers are the strict case: WebTransport has no "proceed anyway" button, so a certificate that is not trusted fails the connection without asking. [Set up TLS certificates](/relay/certificates) shows how to get one for local development and for production.
