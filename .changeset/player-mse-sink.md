---
'moqtail': minor
---

`moqtail/player` exports the `MSE` sink, and `player.sink(video)` now takes an `HTMLVideoElement`. It plays CMAF video and audio through `ManagedMediaSource` or `MediaSource`, appends through one queue per source buffer, keeps 30 s behind and at most 60 s ahead of `currentTime` (the `backBuffer` and `forwardCap` sink options, with `liveEdgeDelay` for its live jumps), seeks upstream when the target isn't buffered, jumps over gaps, and calls `endOfStream()` once every pad ended.
