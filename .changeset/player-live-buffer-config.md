---
'moqtail': minor
---

`liveBuffer()` takes `tolerance`, `stallThreshold`, `minRate`, `jumpFactor` and `jumpTolerance` alongside `catchupRate` and `targetLatency`, so every value the controller tunes with is configurable. The defaults are unchanged.

The playout sink takes a `preroll` option, `player.sink(canvas, { preroll })`: the media seconds it queues before playback starts or restarts after a stall (default 0.5). Lower it together with `stallThreshold` to hold a forward buffer under 0.5 s.
