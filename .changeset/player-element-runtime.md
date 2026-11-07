---
'moqtail': minor
---

`moqtail/player` exports the element runtime: `defineElement`, the `SourceElement`, `TransformElement`, and `SinkElement` base classes, `Pad` with its items, events, and signals, capability bundles, and the `MEDIA_SINK` / `MEDIA_SOURCE` interface keys with `hasInterface`. Elements connect through bounded pad queues with backpressure, and sources are chosen by probe score, then priority.
