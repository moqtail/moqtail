# Grafana dashboard for the relay

`moqtail-relay.json` charts the Prometheus metrics a relay serves at
`/api/v1/metrics` when the API's `metrics` feature is on. Pick one relay or several
from the **Relay** menu at the top.

| Row                    | What it shows                                                                                                                                                 |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Overview               | Version, uptime, connections, tracks, subscriptions, objects out per second, processing time p99, request errors per second                                   |
| Traffic                | Objects and bytes per second in and out (live and FETCH), data streams, dropped objects by reason, subscriptions reset for falling behind                     |
| Object processing time | p99 by stage (ingest, queue, write, total), p50/p95/p99 of the total, a heatmap of the total, time to first object                                            |
| Connections and QUIC   | Connections by state, transport and role; QUIC round-trip time; closes by who closed and the code; connection failures; lost packets                          |
| Requests               | Requests by type and direction, errors by code, request duration p95, requests in flight, upstream requests, limit rejections, control messages, parse errors |
| Cache                  | Cached groups, tracks by origin, groups leaving the cache by cause                                                                                            |
| Accounting zones       | Bytes, objects and requests per namespace prefix in `accounting_zones`                                                                                        |
| Process                | Resident memory and open file descriptors (Linux only), qlog events dropped                                                                                   |

## Scraping the relay

Turn on the `metrics` feature, then point Prometheus at the API. If the API is on
a loopback address, either run Prometheus on the same machine or give the metrics
their own listener with `api_metrics_bind`. Use `api_metrics_token` so the scraper
can read the metrics and nothing else.

```yaml
# relay.yaml
api_enable: true
api_features: [health, state, metrics, latency]
api_metrics_bind: 0.0.0.0:4481
api_metrics_token: '<a long random string>'
api_allow: [127.0.0.1/32, ::1/128, 10.0.0.0/8] # the networks Prometheus scrapes from
```

```yaml
# prometheus.yml
scrape_configs:
  - job_name: moqtail-relay
    scrape_interval: 10s
    metrics_path: /api/v1/metrics
    authorization:
      credentials: '<the api_metrics_token>'
    static_configs:
      - targets: ['relay-1.example:4481', 'relay-2.example:4481']
```

## Importing the dashboard

In Grafana, open **Dashboards → New → Import**, upload `moqtail-relay.json`, and
pick the Prometheus data source. The dashboard's uid is `moqtail-relay`, so
importing it again replaces it.

## Reading it

- Times come from histograms whose buckets double from 10 µs, so a percentile is
  the bound of its bucket: at most twice the real value.
- Request errors are labelled with their REQUEST_ERROR code in decimal: 16 is
  DOES_NOT_EXIST, 1 UNAUTHORIZED, 2 TIMEOUT, 3 NOT_SUPPORTED, 6 GOING_AWAY,
  9 EXCESSIVE_LOAD. The relay dashboard in `apps/dashboard` shows them by name.
- Connection failures and limit rejections stay empty until the first one, since
  the relay only exports those series once they count something.
- The zone panels are empty unless `accounting_zones` is set.
