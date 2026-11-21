---
description: Build a MOQtailPlayer, sink it to a <video> or a <canvas>, start it, control playback and tear it down.
---

# Use `MOQtailPlayer`

Use `MOQtailPlayer` when `<moqtail-video>` doesn't give you enough control: to play on a `<canvas>`, choose your own plugins, add your own elements, or follow the player's states and events from the start. The [tutorial](/ts/tutorial) builds a page with it step by step.

```ts
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({ plugins: [liveBuffer(), abr()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

await player.start()
```

## Create the player

The constructor takes [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions). All of them are optional:

| Option              | Default                     | What it does                                                                                         |
| ------------------- | --------------------------- | ---------------------------------------------------------------------------------------------------- |
| `plugins`           | none                        | Plugins made with `definePlugin()`, such as `liveBuffer()`, `abr()` and `ssts()`                     |
| `catalogTimeoutMs`  | `10000`                     | How long to wait for the catalog before failing with `CATALOG_TIMEOUT`                               |
| `maxCatalogBytes`   | 16 MiB                      | The largest catalog the player reads                                                                 |
| `preferredLanguage` | the browser's language      | The audio language to pick, as a BCP 47 tag                                                          |
| `eventTimelines`    | none                        | The event timelines to deliver as `timelineevent` events: a list of catalog `eventType`s, or `'all'` |
| `objectOrdering`    | a 150 ms window             | How media sources order the objects of each request                                                  |
| `preferWorker`      | `true`                      | Runs decoding and canvas drawing in a worker, see [Run the player in a worker](/ts/player/worker)    |
| `worker`            | the package's worker script | Starts the worker from your own script                                                               |
| `client`            | —                           | `setupOptions`, `transportOptions` and `reconnectAttempts` (default 5) of the session                |

The player installs no plugins by itself. List `liveBuffer()` to hold the live latency and `abr()` to switch renditions, the two plugins `<moqtail-video>` installs.

Every player on a page shares one session with the relay. While it is open, a player for another relay, or with other `client` options or plugins that add other SETUP parameters, throws `UNSUPPORTED_ORIGIN` from `source()`. [How the player works](/guide/concepts/player#one-session-per-page) explains why.

The constructor throws a `TypeError` for a plugin listed twice or a plugin config its factory rejects, and a `PlayerError` with `UNSUPPORTED_ORIGIN` when two plugins set one SETUP parameter to different values.

## Declare the chain

Call the builder methods in this order, all before `init()`:

1. `registerElement()`, for an element of your own, see [Write plugins and elements](/ts/player/plugins).
2. `source()`, once or more.
3. `transform()`, for a transform of your own.
4. `sink()`, once or more.

A call out of order, or after `init()`, throws a `TypeError`.

### Add the source

```ts
player.source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog', { switchFrom: true })
```

`source()` takes the MSF URL of a catalog. A URL that names another track plays only through a source element you register: see [which tracks play](/ts/reference/msf-url#which-tracks-play).

`source()` checks the URL at once, and throws a `PlayerError` that leaves the player `idle`: `UNSUPPORTED_SOURCE` for a URL it can't use, and `UNSUPPORTED_ORIGIN` for another origin than an earlier source's.

With `switchFrom: true`, the media sources switch renditions with SWITCH_FROM, which moves the subscription to the new track, instead of opening a second subscription. The MOQtail relay supports it. Other keys in the options go to the source element.

### Play on a `<video>`

```ts
player.sink(video, { backBuffer: 30 })
```

A `<video>` gets the MSE sink. It plays CMAF and LOCMAF tracks, live and VOD, through `ManagedMediaSource` where the browser has it, and `MediaSource` elsewhere. Its options, in seconds ([`MSEOptions`](/ts/api/player/interfaces/MSEOptions)):

| Option          | Default | What it does                                                                                       |
| --------------- | ------- | -------------------------------------------------------------------------------------------------- |
| `backBuffer`    | `90`    | Media kept behind the playhead                                                                     |
| `forwardCap`    | `60`    | VOD only: the most media buffered ahead of the playhead. Above 0. A live stream keeps all it gets. |
| `liveEdgeDelay` | `0.75`  | How far behind the end of the buffer the sink lands when it jumps to the live edge                 |

A value that is negative or not finite, or a `forwardCap` of 0, throws a `TypeError`.

### Play on a `<canvas>`

```ts
const canvas = document.createElement('canvas')
document.body.append(canvas)

player.sink(canvas, { preroll: 0.5 })
```

A `<canvas>` gets the playout sink. It plays LOC tracks: the player decodes them with WebCodecs, draws the video on the canvas, and plays the audio through an AudioWorklet. When the tracks share the epoch clock, the video stays in sync with the audio. The canvas takes the size of the video frames, so set its displayed size with CSS.

`preroll` is the media, in seconds, queued before playback starts or restarts after a stall: 0.5 by default, at least 0. Lower it together with `liveBuffer()`'s `stallThreshold`, as [Tune live latency](/ts/player/latency#play-at-the-lowest-latency-on-a-canvas) shows.

The canvas sink has limits the `<video>` one doesn't:

- It plays live streams only. A VOD stream fails with `UNSUPPORTED_SOURCE`.
- It seeks forward only. A seek back throws a `TypeError`.
- Pausing and playing again goes on from the live edge, not from where it paused.
- Audio starts only after a user gesture on the page, such as a click that calls `play()`. Video plays meanwhile.
- With the worker, as by default, the canvas is handed to the worker for good. Give each player a new `<canvas>`: a later player on the same one fails with `UNSUPPORTED_SOURCE`. [Run the player in a worker](/ts/player/worker#give-each-player-a-new-canvas) shows how.

## Start playback

`start()` connects, builds the chain, and plays. It is `init()` followed by `play()`:

- `init()` connects to the relay, reads the catalog, builds the chain, and resolves once the player is `ready`, or `ended` when there is nothing to play. It rejects with a `PlayerError` when it fails, for example with `UNSUPPORTED_SOURCE` when no element accepts a track.
- `play()` starts playback on the media sink.

Both `start()` and `init()` can be called more than once: later calls return the same promise.

Call `init()` alone to load the stream and play later, from a click:

```ts
await player.init()
button.addEventListener('click', () => player.play())
```

When the browser refuses to start playback, `play()` and `start()` reject with a `NotAllowedError`, the player moves to `paused`, and it emits `autoplayblocked`. A muted `<video>` is allowed to start. [Handle a blocked autoplay](/ts/player/errors#handle-a-blocked-autoplay) shows what to do.

## Control playback

These calls go to the media sink, so call them once `init()` has resolved. Called earlier, they can fail with `NO_MEDIA_SINK`.

- `pause()` and `play()` pause and resume.
- `seek(seconds)` moves the playhead, in seconds of media time, the clock of `video.currentTime`. It resolves when the seek is done. On a `<video>`, seek anywhere in a VOD stream, or in a live stream's DVR window. On a `<canvas>`, seek forward only.
- `setPlaybackRate(rate)` sets the playback rate. On a live stream, `liveBuffer()` sets the rate itself to hold the latency.

```ts
await player.seek(30)
player.setPlaybackRate(1.5)
await player.pause()
```

## Read what the player holds

| Getter                              | What it holds                                                                                                   |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `state`                             | The player's state, see [Handle states and errors](/ts/player/errors)                                           |
| `mediaSources`                      | The sources that play media, one per switching set, with `getRenditions()`, `switch()` and `selectRenditions()` |
| `mediaSink`                         | The sink that plays: `currentTime`, `buffered`, `paused` and the live range                                     |
| `sources`                           | Every source in the chain, the catalog's included                                                               |
| `client`                            | The shared `MOQtailClient`. A reconnect replaces it and emits `client`.                                         |
| `bufferController`, `abrController` | The plugins' buffer and ABR controllers while they are attached, on live streams                                |
| `options`                           | The options given to the constructor                                                                            |

`isAbrAvailable(source)` says whether a media source switches renditions automatically. [Control adaptive bitrate](/ts/player/abr) uses these to pick renditions.

Listen to the player with `player.on(event, listener)` and stop with `player.off()`. The events are listed in [Handle states and errors](/ts/player/errors) and [`PlayerEvents`](/ts/api/player/interfaces/PlayerEvents).

## Tear it down

```ts
await player.destroy()
```

`destroy()` stops whatever the player is doing, destroys its sinks and then its sources, and lets go of the shared session, which closes when its last player leaves. It works in any state. The state ends at `destroyed`, and every later call fails with `ABORTED`.

## More information

- [`MOQtailPlayer`](/ts/api/player/classes/MOQtailPlayer) and [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions)
- [`MSEOptions`](/ts/api/player/interfaces/MSEOptions) and [`PlayoutOptions`](/ts/api/player/interfaces/PlayoutOptions)
- [`PlayerEvents`](/ts/api/player/interfaces/PlayerEvents) and [`PlayerState`](/ts/api/player/type-aliases/PlayerState)
- [Tune live latency](/ts/player/latency), [Handle states and errors](/ts/player/errors) and [Run the player in a worker](/ts/player/worker)
- [How the player works](/guide/concepts/player)
