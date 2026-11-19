---
'moqtail': minor
---

A `playout` sink, through `player.sink(canvas)`, plays `raw` audio through an AudioWorklet and draws `raw` video on a canvas in sync with it. Its playback rate and forward seeks let `liveBuffer()` hold the configured buffer. It dispatches `waiting` when video or audio arrives later than its frame rate or sample rate allows, and `playing` once it's back. It refuses non-live streams, and a seek back throws.
