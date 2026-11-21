---
description: Connect several relays into a mesh, so a subscriber on one relay gets tracks published on another.
---

# Build a relay mesh

Use a mesh when publishers and subscribers connect to different relays, for example one relay per region. Each relay tells its peers which namespaces it can serve, and sends a request on to the peer that has the best route to the publisher.

Every relay in the mesh needs a certificate. Relays reach each other over raw QUIC on their usual port, so that UDP port must be open between them.

## 1. Give each relay a Hop ID

Setting `hop_id` turns clustering on. Give every relay its own non-zero number:

```yaml
hop_id: 1
```

The relay declares its Hop ID when a session starts, and adds it to the path of every namespace it advertises. That is how a relay spots an advertisement that has already passed through it. Two relays with the same ID discard each other's advertisements as loops.

A relay that only accepts peers and dials none still needs a `hop_id`. Without one, it treats a peer like any other client.

## 2. List the peers

`peers` lists the relays to dial, as `moqt://host:port` URLs:

```yaml
peers: [moqt://relay-b.example.com:4433]
```

The relay dials each peer over raw QUIC at startup and keeps the link open, dialing again a second after it drops. One link carries advertisements both ways, so it is enough for one side to list the other. Two relays may also list each other. They then hold two links, which is fine.

`peers` without `hop_id` stops the relay at startup:

```
error: `peers` needs `hop_id`: without one the relay joins no mesh
```

## 3. Trust the peers' certificates

The relay checks a peer's certificate against the system's trusted roots, as a browser would. A certificate from a public CA, such as Let's Encrypt, works as it is.

For self-signed development certificates, skip the check:

```yaml
peer_no_cert_validation: true
```

::: warning
With the check off, anyone who can answer at a peer's address is taken for that peer. Use it only for development.
:::

## 4. Set relay costs

This step is optional. `relay_cost` is what this relay charges for its own egress. It declares the cost to its peers when a session starts:

```yaml
relay_cost: 10
```

A route's cost adds up the costs of the relays it passes through. A relay picks the cheapest route, then the shortest. A relay with no `relay_cost` counts as 1, so in a mesh without costs the route with the fewest hops wins. Give a relay a higher cost, for example one on metered bandwidth, to steer traffic around it.

## Example: two relays

`relay-a.example.com` dials `relay-b.example.com`. Relay A's `relay-a.yaml`:

```yaml
cert_file: /etc/moqtail/cert.pem
key_file: /etc/moqtail/key.pem
hop_id: 1
peers: [moqt://relay-b.example.com:4433]
api:
  enable: true
```

Relay B's `relay-b.yaml` lists no peers, but still has a Hop ID:

```yaml
cert_file: /etc/moqtail/cert.pem
key_file: /etc/moqtail/key.pem
hop_id: 2
api:
  enable: true
```

Start each one with its file:

```sh
relay --config relay-a.yaml
```

The `api` sections turn on the HTTP API on `127.0.0.1:4480`, which the next section uses to check the mesh. They are optional.

## Check that it works

When the link comes up, relay A logs:

```
Cluster peer moqt://relay-b.example.com:4433 connected to moqt://relay-b.example.com:4433
```

When the peer cannot be reached, it logs `Cluster peer <url> unreachable at <url>: <reason>` and tries again every second.

On either relay, `/api/v1/cluster` shows its Hop ID, each link it dials and each open peer session:

```sh
curl http://127.0.0.1:4480/api/v1/cluster
```

```json
{
  "hop_id": 1,
  "configured_peers": ["moqt://relay-b.example.com:4433"],
  "links": [
    {
      "url": "moqt://relay-b.example.com:4433",
      "up": true,
      "remote": "203.0.113.20:4433",
      "conn_id": 209615788,
      "since_ms": 1795200809493,
      "last_error": null
    }
  ],
  "peers": [
    {
      "conn_id": 209615788,
      "remote": "203.0.113.20:4433",
      "hop_id": 2,
      "relay_cost": 1,
      "adverts": [
        { "namespace": "meshtest", "source_conn_id": 209615790, "hop_path": [0, 1], "cost": 1, "generation": 0 }
      ]
    }
  ]
}
```

A link that is down has `"up": false` and the reason in `last_error`. `adverts` lists the namespaces this relay advertises to the peer, with the path each took. `/api/v1/upstreams` lists the peers among the sources the relay sends requests to, with their round-trip time and how their requests went.

To see a track cross the mesh, publish on relay A and subscribe on relay B with the Rust CLI client, from a checkout of the repository:

```sh
cargo run --bin client -- -c publish-namespace -s moqt://relay-a.example.com:4433 -n meshtest
cargo run --bin client -- -c subscribe -s moqt://relay-b.example.com:4433 -n meshtest
```

The subscriber logs each object it receives, and relay A logs `Advertising Tuple { fields: [meshtest] } to peer ...`.

## When a peer restarts

A relay that shuts down gracefully sends GOAWAY to its peers too. Each peer closes the link at once and withdraws the routes that went through it, so requests take other routes. A relay that dials the peer then dials again: at once if the GOAWAY named a new URI, otherwise after the GOAWAY's timeout, capped at 60 seconds. [Run the relay in production](/relay/production#shut-down-gracefully) covers the shutdown.

## More information

- [How the relay forwards](/guide/concepts/relay), including how the mesh picks routes
- [Configuration file](/relay/reference/config)
- [Monitor the relay](/relay/observability)
