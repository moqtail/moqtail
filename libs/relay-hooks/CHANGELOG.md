# relay-hooks

## 0.19.0

### Minor Changes

- [#477](https://github.com/moqtail/moqtail/pull/477) [`118c7a7`](https://github.com/moqtail/moqtail/commit/118c7a7088c4407a21abc9c68836fded1d388e5c) Thanks [@zafergurel](https://github.com/zafergurel)! - An HTTP API under `/api/v1` for observing and operating the relay, built on its own runtime so it stays off the delivery path, and measured to make no difference to throughput, latency or CPU. It covers connections (with closed-connection history), tracks (with the connection that last published each, as `last_publisher`), namespaces, upstreams, cluster peer links (up or down, when they last changed and why they last dropped), accounting zones, object processing time and time to first object with a slowlog, Prometheus metrics, qlog traces with live follow, and live events over SSE. Each part is switched with `api.features`. Access follows nginx: an address allowlist and separate read, write and metrics tokens. The relay can also publish its own metrics as MoQ tracks.

  - Namespaces and track names are shown the way the library prints them: fields joined by `-`, the track name after `--`, and every byte other than a letter, digit or `_` written as `.xx`, as in `ipc-IPC.2d001-live--video`. Accounting zone prefixes in `api.accounting_zones` use the same form.
  - `GET /api/v1/qlog/{conn_id}` and `/quic` return the trace as one qlog JSON document (`application/qlog+json`) when `Accept` asks for `application/json` or `application/qlog+json`; anything else gets JSON-SEQ, which `?follow=true` needs.
  - `GET /api/v1/meta` serves the values under `api.meta` as written, without a token, so apps can read things like the relay's public host name from the relay itself.
  - Operator actions behind the write token (`actions` feature): GOAWAY to one connection, close one, drain the relay, and change the GOAWAY redirect URI and the log filter while the relay runs.
  - An MCP server at `/api/v1/mcp` (`mcp` feature) so AI agents can query the relay, including a connection's message timeline, and with `api.mcp_write` use the operator actions. It speaks the 2026-07-28 and 2025 protocol revisions, and has its own token, which the relay keeps only as a salted, double SHA-256 hash made with `relay api-token`.
  - A Grafana dashboard for the Prometheus metrics, with its scrape configuration, in `apps/relay-api/grafana`. A clustered relay also writes `moqtail_cluster_peer_up`, `moqtail_cluster_peers` and `moqtail_cluster_adverts`.
