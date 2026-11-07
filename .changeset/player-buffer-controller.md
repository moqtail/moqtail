---
'moqtail': minor
---

Add buffer controllers to the player. `opts.buffer` takes a `BufferController`, attached to the media sink once a live stream is ready and detached on `destroy()` and when the stream turns on-demand; while attached, its optional `backBuffer()` replaces the MSE sink's `backBuffer`. `defaultBufferController({ catchupRate })` keeps live latency at the catalog's `targetLatency ?? buffers.target` with playback rate, within `buffers.min` and `buffers.max`, and jumps toward the live edge when far behind. `player.bufferController` is the attached controller. `<moqtail-video>` uses `defaultBufferController()` when its `config` sets no `buffer`. `TimelineReader` gains `timeToWallclock()`.
