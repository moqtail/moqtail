---
description: Run a relay on your machine and pass objects through it from a publisher to a subscriber, over WebTransport and raw QUIC.
---

# Run your own relay

In this tutorial you run a MOQtail relay on your machine. You then use the Rust CLI client to publish a test track to it and to subscribe to that track, first over WebTransport and then over raw QUIC.

## What you need

- [mkcert](https://github.com/FiloSottile/mkcert#installation), to make a certificate.
- Docker. The relay image is built for `linux/amd64`; on other machines, step 2 shows how to run the relay from source.
- Git and a recent stable [Rust](https://www.rust-lang.org/tools/install) toolchain, to build the CLI client.

You use three terminals: one for the relay, one for the publisher and one for the subscriber.

## 1. Make a certificate

Make a folder for the tutorial, and a certificate in it:

```sh
mkdir moqtail-tutorial && cd moqtail-tutorial
mkcert -install
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 ::1
```

`mkcert -install` creates a local certificate authority and adds it to your system's trust store. It may ask for your password. The second command makes `cert.pem` and `key.pem`, valid for `localhost`. The CLI client trusts the system's store, so it accepts this certificate.

## 2. Start the relay

In the `moqtail-tutorial` folder, run the relay image:

```sh
docker run --rm \
  --user "$(id -u):$(id -g)" \
  -p 4433:4433/udp \
  -v "$PWD/cert.pem:/certs/cert.pem:ro" \
  -v "$PWD/key.pem:/certs/key.pem:ro" \
  ghcr.io/moqtail/relay:latest
```

mkcert made `key.pem` readable only by you, so `--user` runs the relay as you. The last line the relay prints shows where it listens (timestamps are left out on this page):

```
INFO relay::server: moqtail/0.19.0+7412a2f is running on 1 UDP socket(s) -- WebTransport clients: https://localhost:4433  Raw QUIC clients: moqt://localhost:4433
```

The relay takes WebTransport and raw QUIC on the same UDP port, 4433. A warning about UDP socket buffers does not matter here. Leave the relay running.

If you can't run the image, do step 3 first, then start the relay from source in the `moqtail` folder with `cargo run -p relay -- --cert-file ../cert.pem --key-file ../key.pem`.

## 3. Build the CLI client

In a second terminal, in the `moqtail-tutorial` folder, get the code and build the client:

```sh
git clone https://github.com/moqtail/moqtail.git
cd moqtail
cargo build -p client
```

The first build takes a few minutes. The client, `target/debug/client`, publishes, subscribes to and fetches test objects, over either transport.

## 4. Publish a track

In the same terminal:

```sh
./target/debug/client -c publish --server moqt://localhost:4433
```

The client connects over WebTransport, its default, and publishes the track `demo` in the namespace `moqtail`. It sends one object a second, ten objects to a group, each with 1200 bytes of test payload:

```
INFO client::publisher: Track published
INFO client::publisher: Sent object: group=0, subgroup=0, object=0, size=1200 bytes
INFO client::publisher: Sent object: group=0, subgroup=0, object=1, size=1200 bytes
```

Leave it running.

## 5. Subscribe over WebTransport

In a third terminal, go to the `moqtail` folder and subscribe to the track for five seconds:

```sh
./target/debug/client -c subscribe --server moqt://localhost:4433 --duration 5
```

The subscriber asks the relay for `moqtail/demo`, starting with the next object. Each object the publisher sends arrives through the relay:

```
INFO client::subscriber: Subscribed: track=demo track_alias=0 track_properties=[]
INFO client::subscriber: Received object 1: subscription=req=0(demo) group=0, object=2, subgroup=Some(0), priority=128, seq=OK
INFO client::subscriber: Received object 2: subscription=req=0(demo) group=0, object=3, subgroup=Some(0), priority=128, seq=OK
...
INFO client::subscriber: Subscriber finished: received=5, fill=0, errors=0, gaps=0, duplicates=0, filter_violations=0
```

`gaps=0` means that every object arrived, in order. A `StreamClosed` error while the client closes its connection is expected.

In the relay's terminal, a line for each client names the transport it came over:

```
INFO relay::server::session: New session: connection_id=1711044711 transport=webtransport remote=[::ffff:127.0.0.1]:50999
```

## 6. Subscribe over raw QUIC

Run the subscriber again, with `--transport quic`:

```sh
./target/debug/client -c subscribe --server moqt://localhost:4433 --transport quic --duration 5
```

The objects arrive as before. This time the relay's log says `transport=quic`. The relay took both connections on the same port, and it forwards what the WebTransport publisher sends to the raw QUIC subscriber. The publisher takes `--transport quic` too.

## 7. Stop

Press Ctrl-C in the publisher's terminal, then in the relay's. The relay prints `Ctrl-C received, exiting...`, and `--rm` removes the container.

The client has more commands: `publish-namespace`, `subscribe-namespace`, `subscribe-tracks` and `fetch`. [`apps/client/README.md`](https://github.com/moqtail/moqtail/blob/main/apps/client/README.md) lists them with their flags.

## What you built

You made a certificate your machine trusts, ran a relay that serves WebTransport and raw QUIC on one UDP port, and passed a track from a publisher to subscribers over both transports.

## Next steps

- [Build a live player](/ts/tutorial) that runs in the browser. Before you point it at your relay, let the browser accept your mkcert certificate: see [Set up TLS certificates](/relay/certificates).
- [Configure the relay](/relay/configure): the cache, subscriber limits, logs and qlog traces.
- [Monitor the relay](/relay/observability) through its HTTP API and metrics.
