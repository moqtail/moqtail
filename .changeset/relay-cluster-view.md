---
'relay': minor
'relay-api': minor
'relay-hooks': minor
'dashboard': minor
---

The cluster view shows each configured peer's link, up or down, with when it last changed and why it last dropped, and the Prometheus metrics gain `moqtail_cluster_peer_up`, `moqtail_cluster_peers` and `moqtail_cluster_adverts`, written only on a clustered relay. The dashboard gets a Cluster tab with the peer links and the routes advertised to each peer, and the Grafana dashboard a Cluster row.
