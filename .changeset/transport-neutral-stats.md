---
'moqtail-rs': minor
'relay-hooks': minor
'relay-api': minor
'dashboard': patch
---

Report connection statistics and close reasons without QUIC types. `TransportConnection::quic_stats()` is replaced by `transport_stats()`, which returns a `TransportStats` whose QUIC-only fields (RTT, congestion window, congestion events, losses) are `Option`, and `close_reason()` now returns a `CloseReason` instead of `quinn::ConnectionError`.

Breaking for relay-api consumers: a connection's `quic` object is now `stats`, `sent_datagrams`/`received_datagrams` are now `sent_packets`/`received_packets`, and a value the transport cannot measure is `null`. The dashboard shows `–` for it, and the RTT metric skips connections without one.
