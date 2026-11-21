---
description: Set how far behind the live edge the player plays, with the live buffer controller and the sink options.
---

# Tune live latency

Use this page to play a live stream closer to the live edge, or further behind it for fewer stalls. The `liveBuffer()` plugin holds the latency. `<moqtail-video>` installs it by default. `MOQtailPlayer` installs no plugins, so you list it yourself.

## What sets the latency

`liveBuffer()` works on live streams only. It takes its target latency from the first of these that is set:

1. The catalog: `targetLatency`, else `buffers.target`, of the first playing track that sets either (MSF 5.2.9). That track's `buffers.min` and `buffers.max` bound the forward buffer (MSF 5.2.10).
2. The `targetLatency` you pass to `liveBuffer()`, 1 s by default.

A target in the catalog wins over yours.

The controller measures latency with the wallclock values of the stream's media timeline, when there is one (MSF 11.6). Without them, the latency it measures is the forward buffer: the seconds buffered ahead of the playhead.

On every `timeupdate` it compares the forward buffer with its goal, the forward buffer that gives the target latency. The goal never goes below `stallThreshold`, so a target below the stall threshold can't be reached. Then:

- Within `tolerance` of the goal, playback runs at rate 1.
- Above the goal, playback runs at `catchupRate` until it is back.
- Below the goal, playback runs at `2 - catchupRate`, and slower as the buffer runs out below `stallThreshold`, down to `minRate`.
- Above `jumpFactor` times the goal, after a seek back for example, the controller jumps to the goal.

The `<video>` sink has no precise playback rate when it plays through `ManagedMediaSource`, as it does in Safari: every rate change stalls playback for a moment. There the rate stays 1, and the controller jumps to the goal once the forward buffer is more than `jumpTolerance` off it. A jump is a visible skip.

When the viewer seeks back, the controller leaves the rate at 1 until a seek to the live edge.

## Options

`liveBuffer()` takes these options, all in seconds except the rates:

| Option           | Default | Valid values       | Effect                                                                                           |
| ---------------- | ------- | ------------------ | ------------------------------------------------------------------------------------------------ |
| `catchupRate`    | `1.1`   | above 1, below 2   | The rate above the goal. Below it, the rate is `2 - catchupRate`.                                |
| `targetLatency`  | `1`     | above 0            | The latency to hold when the catalog sets no target.                                             |
| `tolerance`      | `0.1`   | 0 or more          | How far the forward buffer may stray from its goal before the rate changes.                      |
| `stallThreshold` | `0.5`   | 0 or more          | The forward buffer below which playback is about to stall. The goal never goes below it.         |
| `minRate`        | `0.6`   | above 0, at most 1 | The slowest rate, as the buffer runs out.                                                        |
| `jumpFactor`     | `4`     | above 1            | A forward buffer this many times its goal is jumped instead of caught up.                        |
| `jumpTolerance`  | `0.5`   | above 0            | On a sink without a precise rate, how far off its goal the forward buffer may get before a jump. |

Every value must also be finite. A value out of range is a `TypeError` when the player is created: the `MOQtailPlayer` constructor throws it, and `<moqtail-video>` fires `error`.

Two sink options work with the controller. Pass them as the second argument of `player.sink()`. `<moqtail-video>` always uses their defaults.

| Sink       | Option          | Default | Effect                                                                                                                                                          |
| ---------- | --------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<video>`  | `liveEdgeDelay` | `0.75`  | How far behind the end of the buffer the sink's own live jumps land: past a gap in the buffer, or when a tab that was hidden has fallen behind the live window. |
| `<canvas>` | `preroll`       | `0.5`   | Seconds of media queued before playback starts, and again after each stall.                                                                                     |

Both take finite values of 0 or more. Lower `preroll` together with `stallThreshold`, or every stall refills 0.5 s.

## Pass the options

With `MOQtailPlayer`, list `liveBuffer()` in `plugins`:

```ts
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({
  plugins: [liveBuffer({ targetLatency: 2 }), abr()],
})
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

await player.start()
```

With `<moqtail-video>`, set `config.plugins` before `src`. The element reads `config` each time it loads a source, and changing it later doesn't restart playback. Your list replaces the default plugins, so list `abr()` too to keep adaptive bitrate:

```ts
import { abr, liveBuffer } from 'moqtail/player'
import { defineMOQtailVideo, type MOQtailVideoElement } from 'moqtail/player/component'

defineMOQtailVideo()

const video = document.querySelector<MOQtailVideoElement>('moqtail-video')!
video.config = { plugins: [liveBuffer({ targetLatency: 2 }), abr()] }
video.src = 'moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog'
```

## Play at the lowest latency on a canvas

The canvas sink plays live LOC tracks, decoded with WebCodecs. This setup holds about 250 ms:

```ts
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const canvas = document.querySelector('canvas')!
const player = new MOQtailPlayer({
  plugins: [liveBuffer({ targetLatency: 0.25, stallThreshold: 0.1, tolerance: 0.05 }), abr()],
})
  .source('moqt://relay.moqtail.dev#msf:moqtail-testsrc--catalog')
  .sink(canvas, { preroll: 0.1 })

await player.start()
```

- `targetLatency: 0.25` asks for a quarter of a second, unless the catalog sets its own target.
- `stallThreshold: 0.1` lets the goal go below the default 0.5 s.
- `tolerance: 0.05` keeps the band around the goal small next to the target.
- `preroll: 0.1` matches `stallThreshold`, so a stall refills 0.1 s, not 0.5 s.

Video starts at once. Audio waits for a user gesture on the page, such as a click. The lower the target, the less buffer there is to hide a late object, so expect more stalls on a link with jitter.

## Play steadier on a video element

For fewer stalls and fewer visible rate changes, hold more latency:

```ts
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({
  plugins: [liveBuffer({ targetLatency: 3, tolerance: 0.3, catchupRate: 1.05, jumpTolerance: 1 }), abr()],
})
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video, { liveEdgeDelay: 1.5 })

await player.start()
```

- `targetLatency: 3` keeps 3 s of media to ride out late objects.
- `tolerance: 0.3` changes the rate less often.
- `catchupRate: 1.05` makes each catch-up slower and harder to notice.
- `jumpTolerance: 1` skips less often in Safari, where the controller jumps instead.
- `liveEdgeDelay: 1.5` leaves more buffer after the sink jumps a gap.

## More information

- [`liveBuffer`](/ts/api/player/variables/liveBuffer) and [`LiveBufferConfig`](/ts/api/player/interfaces/LiveBufferConfig)
- [`MSEOptions`](/ts/api/player/interfaces/MSEOptions) and [`PlayoutOptions`](/ts/api/player/interfaces/PlayoutOptions)
- [`BufferController`](/ts/api/player/interfaces/BufferController), to write your own: see [Write plugins and elements](/ts/player/plugins)
- [How the player works](/guide/concepts/player)
