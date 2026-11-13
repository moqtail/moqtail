---
'relay': minor
'relay-api': minor
'relay-hooks': minor
---

An HTTP API under `/api/v1` for observing and operating the relay, built on its own runtime so it stays off the delivery path, and measured to make no difference to throughput, latency or CPU. It covers connections (with closed-connection history), tracks, namespaces, upstreams, cluster, accounting zones, object processing time and time to first object with a slowlog, Prometheus metrics, qlog traces with live follow, and live events over SSE. Each part is switched with `api_features`. Access follows nginx: an address allowlist and separate read, write and metrics tokens. The relay can also publish its own metrics as MoQ tracks.
