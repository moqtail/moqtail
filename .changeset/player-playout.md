---
'moqtail': minor
---

A `playout` sink, through `player.sink(canvas)`, plays `raw` audio through an AudioWorklet and draws `raw` video on a canvas in sync with it. It keeps three decoded frames, and its `buffered` counts what the decoder holds encoded, so `liveBuffer()` holds the configured buffer through its playback rate, on WebKit too, and its forward seeks. It dispatches `waiting` when video or audio arrives later than its frame rate or sample rate allows, and `playing` once it's back. It refuses non-live streams, and a seek back throws.
