---
description: The relay's HTTP API under /api/v1, its endpoints by feature, how requests are authorized, the settings it refuses at startup and its MCP tools.
---

# HTTP API

The relay serves the API when `api.enable` is `true`, at `http://127.0.0.1:4480/api/v1` unless `api.bind` says otherwise. Its settings are the `api` keys of the [configuration file](/relay/reference/config). Builds without the `observability` Cargo feature have no API.

Every path starts with `/api/v1`. Responses are JSON unless an endpoint says otherwise.

## Features

Each endpoint belongs to a feature, and `api.features` lists the features that run.

| Feature   | On by default | Serves                                                                 |
| --------- | ------------- | ---------------------------------------------------------------------- |
| `health`  | yes           | The endpoint index, liveness, readiness and `meta`                     |
| `state`   | yes           | Snapshots of the relay: configuration, connections, tracks, namespaces |
| `metrics` | yes           | Prometheus metrics                                                     |
| `latency` | yes           | Object processing time and the slowlog                                 |
| `qlog`    | yes           | The qlog trace files, when `qlog_folder` is set                        |
| `events`  | no            | A live stream of qlog events                                           |
| `actions` | no            | Endpoints that change the running relay. Needs `write_token`.          |
| `mcp`     | no            | The MCP server for AI agents. Needs `mcp_token_hash`.                  |

An endpoint whose feature is off answers 404 with the error `feature_off`.

## Endpoints

Paths are relative to `/api/v1`. `{conn_id}` and `{track_id}` are the numeric ids the list endpoints return. The connection and track lists are at most one second old.

### `health`

| Method | Path      | Returns                                                                                                                   |
| ------ | --------- | ------------------------------------------------------------------------------------------------------------------------- |
| GET    | `/`       | The relay's `version`, the features that run, and every endpoint with its method, path, feature and whether it is enabled |
| GET    | `/health` | `{"status": "ok"}` while the process answers, also while it drains                                                        |
| GET    | `/ready`  | `{"ready": true}`, or 503 with `"reason": "starting"` before the QUIC listeners accept and `"draining"` while it drains   |
| GET    | `/meta`   | `api.meta` as written                                                                                                     |

### `state`

| Method | Path                     | Returns                                                                                                                                                      |
| ------ | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| GET    | `/config`                | The settings the relay runs on, without the tokens                                                                                                           |
| GET    | `/overview`              | Version, process id, start time, connections by state, track, subscription and cached group counts, request results, and traffic totals when `metrics` is on |
| GET    | `/connections`           | A [page](#list-parameters) of open connections, or of closed ones with `closed=true`                                                                         |
| GET    | `/connections/{conn_id}` | One open or recently closed connection: SETUP options, namespaces, tracks, subscriptions, QUIC statistics, close reason                                      |
| GET    | `/tracks`                | A [page](#list-parameters) of tracks with their publishers, subscriber count, cached groups and processing time                                              |
| GET    | `/tracks/{track_id}`     | One track with every subscription to it and the groups in its cache                                                                                          |
| GET    | `/namespaces`            | Each announced namespace with the connections announcing it, and the namespace subscriptions the relay holds                                                 |
| GET    | `/upstreams`             | The open publishers and cluster peers the relay can request from, with round-trip time and request results                                                   |
| GET    | `/zones`                 | Traffic and request results per `api.accounting_zones` prefix, then `unzoned`. Empty without zones.                                                          |
| GET    | `/cluster`               | The relay's mesh identity, its links to peers, and what it advertises to each                                                                                |

### `metrics`

| Method | Path       | Returns                                                                                                                      |
| ------ | ---------- | ---------------------------------------------------------------------------------------------------------------------------- |
| GET    | `/metrics` | Every counter in the Prometheus text format, `text/plain; version=0.0.4`. `filter` keeps the metrics whose name contains it. |

Per-connection and per-track numbers are not metrics. They are in `/connections` and `/tracks`.

### `latency`

| Method | Path            | Returns                                                                                                                                                                                                         |
| ------ | --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GET    | `/latency`      | Percentiles in milliseconds of each processing stage (ingest, queue, write, total) and of time to first object, with the slowest tracks and subscriptions. `window` is `10s`, `60s` (default), `5m` or `start`. |
| GET    | `/latency/slow` | The slowlog, newest first, with its two thresholds                                                                                                                                                              |

### `qlog`

| Method | Path                   | Returns                                                                                                       |
| ------ | ---------------------- | ------------------------------------------------------------------------------------------------------------- |
| GET    | `/qlog`                | The trace files in `qlog_folder`, newest first, with their connection, kind, size and whether they still grow |
| GET    | `/qlog/{conn_id}`      | The connection's MoQT trace                                                                                   |
| GET    | `/qlog/{conn_id}/quic` | The connection's QUIC packet trace. Only debug builds of the relay write one.                                 |

A trace comes as JSON-SEQ, `application/qlog+json-seq`. A request whose `Accept` names `application/json` or `application/qlog+json` gets one JSON document instead. On `/qlog/{conn_id}`, `since` keeps the events at or after a time in milliseconds since the Unix epoch, and `follow=true` keeps the response open and sends each new event until the connection closes. `follow` only streams JSON-SEQ.

Without `qlog_folder`, these endpoints answer 409 `qlog_off`.

### `events`

| Method | Path      | Returns                                                              |
| ------ | --------- | -------------------------------------------------------------------- |
| GET    | `/events` | Every connection's qlog events as they happen, as Server-Sent Events |

| Event    | Data                                                         |
| -------- | ------------------------------------------------------------ |
| `qlog`   | The qlog event, with `conn_id` added as its first field      |
| `closed` | `{"conn_id": …}` when a connection closes                    |
| `lagged` | `{"missed": …}`, the events a listener that fell behind lost |

`conn` keeps one connection's events, and `name` the events whose name starts with it, such as `moqt:control_message`. Without `qlog_folder`, the relay builds events only while someone listens.

### `actions`

Every endpoint here needs the write token, and the relay logs each change with the address that asked for it.

| Method | Path                            | Body                             | Does                                                                                                                                                                     |
| ------ | ------------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| DELETE | `/connections/{conn_id}`        |                                  | Closes the connection at once. 204.                                                                                                                                      |
| POST   | `/connections/{conn_id}/goaway` |                                  | Sends the client GOAWAY, and closes the connection if it is still open after `timeout_ms`. 202.                                                                          |
| POST   | `/drain`                        |                                  | Refuses new requests and sends every client GOAWAY, closing those still open after `timeout_ms`. Answers `{"told": …, "timeout_ms": …}`. Lasts until the relay restarts. |
| GET    | `/config/redirect_uri`          |                                  | `{"uri": …}`, the URI GOAWAY sends clients to                                                                                                                            |
| PUT    | `/config/redirect_uri`          | `{"uri": "moqt://…"}`            | Replaces it. `null` or `""` sends none. At most 8192 bytes.                                                                                                              |
| GET    | `/logging`                      |                                  | `{"filter": …}`, the current log filter                                                                                                                                  |
| PUT    | `/logging`                      | `{"filter": "info,relay=debug"}` | Replaces the log filter, written as for `RUST_LOG`                                                                                                                       |

`timeout_ms` is a query parameter: 10000 when left out, at most 600000.

### `mcp`

| Method | Path   | Serves                                                                           |
| ------ | ------ | -------------------------------------------------------------------------------- |
| POST   | `/mcp` | The MCP server, over the Streamable HTTP transport. See [MCP tools](#mcp-tools). |

## List parameters

`/connections` and `/tracks` return one page:

```json
{ "total": 42, "offset": 0, "limit": 100, "items": [] }
```

| Parameter | Description                                                                                                                                                                                                                 |
| --------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sort`    | A sort key. A leading `-` sorts in descending order: `sort=-bytes_out`.                                                                                                                                                     |
| `limit`   | Items per page. 100 when left out, at most 1000.                                                                                                                                                                            |
| `offset`  | Items to skip. 0 when left out.                                                                                                                                                                                             |
| `prefix`  | Only entries in a namespace that starts with this text. Namespaces are written with their fields joined by `-`, such as `moqtail-ch07`. A connection matches when it announces, publishes or subscribes to something there. |
| `closed`  | `/connections` only. `true` lists the recently closed connections, up to `api.closed_connection_history`.                                                                                                                   |

| Endpoint       | Sort keys                                                                                  |
| -------------- | ------------------------------------------------------------------------------------------ |
| `/connections` | `conn_id`, `connected_at`, `rtt`, `bytes_out`, `bytes_in`, `lost_packets`, `subscriptions` |
| `/tracks`      | `track_id`, `subscribers`, `publishers`, `cached_groups`                                   |

## Authorization

Every request passes two checks before it reaches an endpoint: the client's address, then its token.

### Address

On a loopback `api.bind`, every client is allowed. On a network address, the client's IP has to be in `api.allow`, or the request gets 403 `address_not_allowed`. `api.metrics_bind` is checked against the same list. An IPv4 client on an IPv6 socket is matched as IPv4.

### Tokens

A token goes in the `Authorization` header: `Authorization: Bearer <token>`. A token set to `""` counts as unset.

| Request                          | Accepted tokens                                                                                                                               |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /health`, `/ready`, `/meta` | None needed, ever                                                                                                                             |
| Other `GET` and `HEAD`           | Open without `read_token`. With it, `read_token` or `write_token`.                                                                            |
| `GET /metrics`                   | As other reads, and also `metrics_token`. With `metrics_token` set and no `read_token`, the metrics need a token while other reads stay open. |
| `POST`, `PUT`, `DELETE`          | `write_token`                                                                                                                                 |
| `/mcp`                           | Only the MCP token, the one `mcp_token_hash` was made from. No other token works there, and it works nowhere else.                            |

A request without a needed token gets 401 `token_required`, and one with a wrong token 403 `token_rejected`. The tokens apply on the metrics listener too.

### Browsers

A request whose `Origin` is in `api.cors_origins` gets CORS headers, and its preflight `OPTIONS` gets 204. A preflight from any other origin gets 403.

`/mcp` refuses any request that has an `Origin` not in `api.cors_origins`, with 403 `origin_not_allowed`. On a loopback `api.bind`, it also needs a `Host` of `localhost`, `127.0.0.1` or `::1`. Both guard against DNS rebinding.

## Errors

An error answers with a status and a JSON body: a stable `error` code, and often a `detail` sentence that says what to change.

```json
{ "error": "feature_off", "detail": "add `actions` to api.features to turn this endpoint on" }
```

| Status | `error`                                                                       |
| ------ | ----------------------------------------------------------------------------- |
| 400    | `bad_query`, `unknown_sort_key`, `unknown_window`, `bad_body`, `bad_filter`   |
| 401    | `token_required`                                                              |
| 403    | `address_not_allowed`, `origin_not_allowed`, `token_rejected`                 |
| 404    | `not_found`, `feature_off`, `unknown_connection`, `unknown_track`, `no_trace` |
| 406    | `follow_needs_json_seq`: `follow` asked for with `Accept: application/json`   |
| 409    | `qlog_off`, `events_off`                                                      |
| 500    | `qlog_unreadable`                                                             |

## Startup refusals

With `api.enable` set, the relay refuses to start, and exits with code 1 and the reason, when:

| Setting                | Refused when                                                                                                                                                                         |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `features`             | It names a feature that does not exist                                                                                                                                               |
| `features`             | It has `actions` and `write_token` is not set                                                                                                                                        |
| `features`             | It has `mcp` and `mcp_token_hash` is not set                                                                                                                                         |
| `mcp_token_hash`       | It is not a `sha256d$<salt>$<digest>` line as [`relay api-token`](/relay/reference/cli#api-token) prints it                                                                          |
| `allow`                | An entry is neither an IP address nor a range, such as `10.0.0.0/8`, with a prefix length its family allows                                                                          |
| `allow`                | It is empty, and `bind` or `metrics_bind` is a network address                                                                                                                       |
| `allow`                | An entry with prefix length 0, such as `0.0.0.0/0` or `::/0`, opens a network `bind` with no `read_token`, or a network `metrics_bind` with neither `read_token` nor `metrics_token` |
| `latency_detail`       | It is not `relay`, `track` or `subscription`                                                                                                                                         |
| `metrics_resource_id`  | `metrics_track_enable` is on and the id is not 16 lowercase hexadecimal characters                                                                                                   |
| `bind`, `metrics_bind` | The address cannot be bound, for example because the port is in use                                                                                                                  |

The relay starts, with a warning in its log, when:

- `allow` lists other ranges than loopback, but `bind` is a loopback address, where `allow` is ignored.
- `bind` is a network address and `allow` is left at loopback only, so remote clients get 403.
- `bind` is a network address with no `read_token`, so the ranges in `allow` read without a token.
- `mcp_write` is on without the `actions` feature, where it has no effect.

## MCP tools

The MCP server offers a tool only when its feature is on. The write tools also need `api.mcp_write`. A tool with a `limit` returns 50 items when it is left out.

| Tool                  | Feature                        | Arguments                                               | Returns                                                                                                                 |
| --------------------- | ------------------------------ | ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `relay_overview`      | `state`                        |                                                         | What `/overview` returns                                                                                                |
| `list_connections`    | `state`                        | `closed`, `sort`, `limit` (max 100), `offset`, `prefix` | A page of connections, as `/connections`                                                                                |
| `get_connection`      | `state`                        | `conn_id`                                               | One connection in full                                                                                                  |
| `connection_timeline` | `qlog`, with `qlog_folder` set | `conn_id`, `limit` (max 200)                            | The control messages a connection exchanged, each request with its responses, stream resets, parse errors and the close |
| `list_tracks`         | `state`                        | `sort`, `limit` (max 100), `offset`, `prefix`           | A page of tracks, as `/tracks`                                                                                          |
| `get_track`           | `state`                        | `track_id`                                              | One track with its subscriptions and cache                                                                              |
| `namespaces`          | `state`                        |                                                         | What `/namespaces` returns                                                                                              |
| `upstreams`           | `state`                        |                                                         | What `/upstreams` returns                                                                                               |
| `zones`               | `state`                        |                                                         | What `/zones` returns                                                                                                   |
| `cluster`             | `state`                        |                                                         | What `/cluster` returns                                                                                                 |
| `latency_report`      | `latency`                      | `window`                                                | What `/latency` returns                                                                                                 |
| `slowlog`             | `latency`                      | `limit` (max 100)                                       | The newest slowlog entries                                                                                              |
| `send_goaway`         | `actions`, write               | `conn_id`, `timeout_ms`                                 | Sends one client GOAWAY, as `POST /connections/{conn_id}/goaway`                                                        |
| `close_connection`    | `actions`, write               | `conn_id`                                               | Closes one connection at once                                                                                           |
| `drain_relay`         | `actions`, write               | `timeout_ms`                                            | Drains the relay, as `POST /drain`                                                                                      |
| `set_log_filter`      | `actions`, write               | `filter`                                                | Replaces the log filter                                                                                                 |
| `set_redirect_uri`    | `actions`, write               | `uri`                                                   | Replaces the GOAWAY redirect URI                                                                                        |

A failed call returns a tool error with the reason, which the agent can read and act on.

## Related

- [Monitor the relay](/relay/observability)
- [Connect an AI agent over MCP](/relay/mcp)
- [Configuration file](/relay/reference/config)
