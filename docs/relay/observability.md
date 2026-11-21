---
description: Turn on the relay's HTTP API, secure it, and watch the relay with curl, Prometheus and Grafana, qlog traces, the slowlog and live events.
---

# Monitor the relay

The relay serves an HTTP API under `/api/v1` with its state, Prometheus metrics, qlog traces and the time objects take to pass through it. The API is off until you turn it on. It is part of the container image and of default builds.

## Turn on the API

Add an `api` section to the config file:

```yaml
api:
  enable: true
```

The API listens on `127.0.0.1:4480`, so only programs on the same machine can reach it. The relay logs `API listening on http://127.0.0.1:4480/api/v1` at startup. Ask it what it serves:

```sh
curl http://127.0.0.1:4480/api/v1
```

The answer lists every endpoint with the feature it belongs to and whether that feature is on. These features run by default:

| Feature   | What it serves                                                               |
| --------- | ---------------------------------------------------------------------------- |
| `health`  | The endpoint list, liveness, readiness and `meta`                            |
| `state`   | Overview, connections, tracks, namespaces, upstreams, mesh, zones and config |
| `metrics` | Prometheus metrics                                                           |
| `latency` | Object processing time and the slowlog                                       |
| `qlog`    | The qlog trace files                                                         |

Three more are off by default: `events` (live events), `actions` (endpoints that change the relay) and `mcp` (the [MCP server](/relay/mcp)). Setting `features` replaces the default list, so name every feature you want:

```yaml
api:
  enable: true
  features: [health, state, metrics, latency, qlog, events]
```

An endpoint whose feature is off answers `404` with `feature_off`.

Look around with curl:

```sh
curl http://127.0.0.1:4480/api/v1/overview
curl http://127.0.0.1:4480/api/v1/connections
curl http://127.0.0.1:4480/api/v1/tracks
```

[HTTP API](/relay/reference/http-api) lists every endpoint.

## Open the API to the network

To reach the API from other machines, bind it to a network address, say which clients may connect, and set tokens:

```yaml
api:
  enable: true
  bind: 0.0.0.0:4480
  allow: [10.0.0.0/8]
  read_token: '<a long random string>'
  write_token: '<another long random string>'
```

- `allow` lists the client address ranges that may connect. Others get `403`. Left out, it allows loopback only. On a loopback `bind` it is ignored. Quote IPv6 ranges that start with `:`, such as `'::1/128'`, or YAML cannot read the list.
- `read_token` is needed for every read except `/api/v1/health`, `/api/v1/ready` and `/api/v1/meta`, which load balancers and apps read without one. Without a `read_token`, reads are open to every address in `allow`.
- `write_token` is needed for the `actions` endpoints, and also allows reads.

Make each token with, for example, `openssl rand -hex 32`. Send it as a bearer token:

```sh
curl -H "Authorization: Bearer $READ_TOKEN" http://relay.example.com:4480/api/v1/overview
```

A missing token gets `401`, a wrong one `403`. The API speaks plain HTTP, so outside a private network, put it behind a reverse proxy with TLS.

The relay refuses to start, rather than expose the API, when `allow` is empty on a network `bind`, when `allow` covers every address and no `read_token` is set, when `actions` is on without a `write_token`, and when `mcp` is on without an `mcp_token_hash`. [Startup refusals](/relay/reference/http-api#startup-refusals) lists them all.

To call the API from a web page, such as the [relay dashboard](https://github.com/moqtail/moqtail/tree/main/apps/dashboard), list the page's origin in `cors_origins`:

```yaml
api:
  cors_origins: [https://dashboard.example.com]
```

### In a container

Inside a container, `127.0.0.1` is the container's own loopback. Bind the API to `0.0.0.0:4480` and publish the TCP port to the host only, with `-p 127.0.0.1:4480:4480`. Requests from the host then arrive from the Docker bridge's gateway, `172.17.0.1` on Docker's default network, so allow that address:

```yaml
api:
  enable: true
  bind: 0.0.0.0:4480
  allow: [172.17.0.1/32]
```

## Scrape the metrics with Prometheus

The `metrics` feature serves Prometheus metrics at `/api/v1/metrics`. To open only the metrics to your monitoring network, give them a listener of their own with `metrics_bind`, and a token that reads the metrics and nothing else with `metrics_token`:

```yaml
api:
  enable: true
  metrics_bind: 0.0.0.0:4481
  metrics_token: '<a long random string>'
  allow: [127.0.0.1/32, 10.0.0.0/8]
```

The rest of the API stays on `127.0.0.1:4480`. The metrics listener serves `/api/v1/metrics` only, and takes the same `allow` list. Then point Prometheus at it, in `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: moqtail-relay
    scrape_interval: 10s
    metrics_path: /api/v1/metrics
    authorization:
      credentials: '<the metrics_token>'
    static_configs:
      - targets: ['relay-1.example.com:4481', 'relay-2.example.com:4481']
```

## Import the Grafana dashboard

The repository ships a Grafana dashboard for these metrics, `moqtail-relay.json` in [`apps/relay-api/grafana`](https://github.com/moqtail/moqtail/tree/main/apps/relay-api/grafana). It charts traffic, processing time, connections and QUIC, requests, the cache, accounting zones and the mesh, for one relay or several.

1. Download `moqtail-relay.json`.
2. In Grafana, open **Dashboards → New → Import** and upload the file.
3. Pick your Prometheus data source.

The dashboard's uid is `moqtail-relay`, so importing a newer version replaces the old one. Its percentiles are the upper bounds of histogram buckets that double from 10 µs, so each is at most twice the real value. The mesh panels stay empty unless the relay has a `hop_id`, and the zone panels unless `accounting_zones` is set.

## Record qlog traces

qlog traces record the control messages of every connection, for when you need to see why a request failed. Turn them on with `qlog_folder`, a top-level key outside `api`:

```yaml
qlog_folder: /var/log/moqtail/qlog
qlog_max_file_bytes: 104857600
```

The relay writes one `.sqlog` file per connection into the folder. A long-lived publisher's file keeps growing, so `qlog_max_file_bytes` stops each file at about that size. Debug builds also write a QUIC packet trace per connection, ending in `_quic.sqlog`.

With the API's `qlog` feature, fetch the traces over HTTP. List them, newest first:

```sh
curl http://127.0.0.1:4480/api/v1/qlog
```

Then fetch one by its `conn_id`:

```sh
curl http://127.0.0.1:4480/api/v1/qlog/209615789
```

The trace comes as JSON-SEQ, as written. Add `?follow=true` to keep the response open and get each new event while the connection lasts. Send `Accept: application/qlog+json` to get one JSON document instead, for tools that read that form. `/api/v1/qlog/<conn_id>/quic` serves the QUIC trace of a debug build.

## Measure processing time

The `latency` feature measures how long objects take inside the relay, from arrival to written, in stages: ingest, queue, write and total. It also measures how long a new subscription waits for its first object.

```sh
curl "http://127.0.0.1:4480/api/v1/latency?window=60s"
```

The answer gives percentiles in milliseconds for each stage, and the slowest tracks and subscriptions. `window` is `10s`, `60s` (the default), `5m` or `start`.

Two settings under `api` control the cost of measuring:

- `latency_detail` keeps times for the whole relay (`relay`), also per track (`track`), or also per subscription (`subscription`, the default).
- `latency_sample: N` measures every Nth object only.

### Read the slowlog

The slowlog keeps the slowest recent deliveries, newest first, with the track, connection and request of each:

```sh
curl http://127.0.0.1:4480/api/v1/latency/slow
```

A delivery slower than `slowlog_threshold` (20 ms) goes in, and so does a subscription that waits longer than `slowlog_first_object_threshold` (1 s) for its first object. The last `slowlog_size` (1000) entries are kept.

## Watch live events

The `events` feature streams every connection's qlog events as they happen, as Server-Sent Events. It works with or without `qlog_folder`.

```sh
curl -N "http://127.0.0.1:4480/api/v1/events?name=moqt:control_message"
```

Each event named `qlog` holds one qlog event as JSON, with the `conn_id` of its connection. A `closed` event says a connection closed, and `lagged` says how many events a slow listener missed. `?conn=<conn_id>` keeps one connection's events, and `?name=` the events whose name starts with the given text.

## Count traffic per tenant

`accounting_zones` lists namespace prefixes whose traffic and requests are counted separately, for example one per tenant. Write a prefix's fields joined by `-`:

```yaml
api:
  accounting_zones: [tenant1, tenant2-live]
```

A namespace counts toward the longest prefix it falls under. Everything else counts as `unzoned`. `/api/v1/zones` shows the counts, and the metrics carry them too.

## Publish the metrics as MOQT tracks

With `metrics_track_enable`, the relay also publishes its own metrics as MOQT tracks, so any MOQT client can subscribe to them, through this relay or a mesh peer:

```yaml
api:
  enable: true
  metrics_track_enable: true
  metrics_resource_id: 0123456789abcdef
```

The tracks are under the namespace `metrics.moq.arpa/v1/<metrics_resource_id>`: a `catalog` that describes each metric, `4` with a sample every second, and `6` with a sample every ten seconds. `metrics_resource_id` is 16 lowercase hexadecimal characters. Without it, the relay makes up a new one each time it starts. Each sample is a group, so a client can also fetch recent samples from the relay's cache. To read track `4` with the Rust CLI client:

```sh
cargo run --bin client -- -c subscribe -s moqt://localhost:4433 -n metrics.moq.arpa/v1/0123456789abcdef -T 4
```

## More information

- [HTTP API](/relay/reference/http-api), every endpoint by feature
- [Configuration file](/relay/reference/config), every `api` key with its default
- [Connect an AI agent](/relay/mcp)
- [Run the relay in production](/relay/production)
