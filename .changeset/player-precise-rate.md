---
'moqtail': minor
---

`MediaSinkElement` has a `preciseRate` flag: whether `setPlaybackRate()` keeps the clock running. `liveBuffer()` holds latency with jumps only on a sink without one, which the `mse` sink is on WebKit. A custom media sink sets it.
