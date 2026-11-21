---
description: Follow the player's states, tell fatal errors from warnings, and react to reconnects and blocked autoplay.
---

# Handle states and errors

Use this page to show the viewer what the player is doing, and to react when something goes wrong.

## Follow the state

`player.state` holds one of these states. On every change, the player emits `statechange` with the new and the previous state.

| State          | Meaning                                                                                      |
| -------------- | -------------------------------------------------------------------------------------------- |
| `idle`         | Created. `init()` wasn't called yet.                                                         |
| `connecting`   | `init()` is connecting to the relay and building the chain.                                  |
| `ready`        | The chain is ready to play.                                                                  |
| `playing`      | Playing.                                                                                     |
| `paused`       | Paused, by `pause()` or by a blocked autoplay.                                               |
| `stalled`      | Waiting for data.                                                                            |
| `reconnecting` | The session was lost and the player is reconnecting. It returns to the state it interrupted. |
| `ended`        | The stream ended, or there was nothing to play.                                              |
| `error`        | A fatal error happened. Final: only `destroy()` leaves it.                                   |
| `destroyed`    | `destroy()` finished. Final.                                                                 |

The states move like this:

- `idle` goes to `connecting` on `init()`, then to `ready`, or to `ended` when there is nothing to play.
- `ready`, `playing`, `paused` and `stalled` follow the media sink: what its `<video>` or canvas reports.
- `ended` goes back to `playing` when you seek back into the stream.
- From `ready` on, a lost session moves to `reconnecting`, and back once the session is restored. From `ended`, only a seek that needs the session does.
- Any state from `connecting` on can move to `error`. Every state can move to `destroyed`.

On entering `playing`, `paused`, `stalled` and `ended`, the player also emits the event of the same name (`pause` for `paused`).

## Listen for errors and warnings

The player reports problems with two events. Both carry a `PlayerError`:

- `error` is fatal. The player moves to `error` and pauses. Nothing recovers it: destroy it and create a new one.
- `warning` isn't. Playback goes on, but something was left out, such as a track.

A `PlayerError` has a `code`, a `category` (`source`, `network`, `media` or `aborted`), the name of the `element` it came from when there is one, a `message`, and the original error as `cause` when there is one.

::: code-group

```html [index.html]
<video playsinline></video>
<p id="status"></p>
<button id="play" hidden>Play</button>
```

```ts [main.ts]
import { MOQtailPlayer, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const status = document.querySelector<HTMLParagraphElement>('#status')!
const playButton = document.querySelector<HTMLButtonElement>('#play')!

const player = new MOQtailPlayer({ plugins: [liveBuffer()], client: { reconnectAttempts: 10 } })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

player.on('statechange', (state, previous) => console.log(`${previous} → ${state}`))
player.on('warning', (warning) => console.warn(warning.code, warning.element, warning.message))
player.on('error', (error) => {
  status.textContent = error.category === 'network' ? 'The connection was lost.' : `Playback failed: ${error.message}`
})
player.on('reconnecting', (attempt) => {
  status.textContent = `Reconnecting (attempt ${attempt})…`
})
player.on('reconnected', () => {
  status.textContent = ''
})
player.on('autoplayblocked', () => {
  playButton.hidden = false
})
playButton.addEventListener('click', () => {
  playButton.hidden = true
  void player.play()
})

// A failure is also the `error` event; a blocked autoplay is `autoplayblocked`
player.start().catch(() => {})
```

:::

Not every error reaches the `error` event:

- The constructor and `source()` throw at once, and the player stays `idle`.
- `init()` and `start()` reject with the error the `error` event carries.
- `seek()` rejects with `SEEK_FAILED` or `ABORTED`, and playback goes on.
- `play()`, `pause()`, `seek()` and `setPlaybackRate()` without a media sink reject or throw with `NO_MEDIA_SINK`.
- After `destroy()`, every call rejects or throws with `ABORTED`.

So wrap the builder calls when the URL comes from the viewer:

```ts
import { MOQtailPlayer, PlayerError } from 'moqtail/player'

try {
  new MOQtailPlayer().source(url).sink(video)
} catch (error) {
  if (error instanceof PlayerError) console.error(error.code, error.message)
  else throw error
}
```

## Error codes

These codes are errors:

| Code                 | Category  | Raised when                                                                                                                                                                                                                                                                                                                                                                                                    | What to do                                                                                                                                         |
| -------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `UNSUPPORTED_SOURCE` | `source`  | `source()` gets a URL it can't use, such as one that isn't an [MSF URL](/ts/reference/msf-url). No element accepts a track: [which tracks play](/ts/reference/msf-url#which-tracks-play). The catalog lists no track the browser can play, or none for its video or its audio. An element fails to start, such as the canvas sink on a stream that isn't live, or on a canvas an earlier player's worker took. | Check the URL, and that the browser plays the stream's codecs. Give each player [its own canvas](/ts/player/worker#give-each-player-a-new-canvas). |
| `UNSUPPORTED_ORIGIN` | `source`  | Players share one client per page, so a player for another origin, or with other `client` options or plugin setup parameters, can't connect while it is in use. Also when two plugins of one player set a setup parameter to different values.                                                                                                                                                                 | Give every player on the page the same origin, `client` options and plugins. Destroy the others before you switch origin.                          |
| `CATALOG_TIMEOUT`    | `source`  | No usable catalog, or for a track URL without one no answer or first object, arrived within `catalogTimeoutMs`, 10 000 ms by default.                                                                                                                                                                                                                                                                          | Check the namespace in the URL, and that the publisher is live. Raise `catalogTimeoutMs` for a slow start.                                         |
| `SESSION_LOST`       | `network` | The player couldn't connect, or couldn't reconnect within `client.reconnectAttempts` attempts.                                                                                                                                                                                                                                                                                                                 | Check the relay's address and certificate. [Create a new player](#reconnect-after-a-lost-session) to try again.                                    |
| `MEDIA_FAILURE`      | `media`   | The browser couldn't play the media: the `<video>` sink couldn't create a source buffer, or playback failed again after it rebuilt the media source. The canvas sink couldn't start its audio output. An engine or the worker failed.                                                                                                                                                                          | Check that the browser plays the stream's codecs.                                                                                                  |
| `SEEK_FAILED`        | `source`  | `seek()` rejects: a source couldn't seek there.                                                                                                                                                                                                                                                                                                                                                                | Seek within the `<video>`'s `seekable` range.                                                                                                      |
| `NO_MEDIA_SINK`      | `media`   | No declared sink can play any track of the catalog, such as a canvas for a stream that has no LOC tracks. A media sink was left with no pads. A playback call came without a media sink.                                                                                                                                                                                                                       | Declare a sink that fits the stream: a `<video>` for CMAF and LOCMAF, a `<canvas>` for LOC.                                                        |
| `ABORTED`            | `aborted` | A call came after `destroy()`, or a newer `seek()` replaced one that was running.                                                                                                                                                                                                                                                                                                                              | Nothing: it is expected.                                                                                                                           |

These codes are warnings:

| Code                     | Raised when                                                                                                                                                         | What to do                                                                        |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `PAD_UNLINKED`           | No transform or sink takes a pad, so its track doesn't play.                                                                                                        | Add a transform or sink whose `in` capabilities match the pad.                    |
| `TRACK_SKIPPED`          | The player skipped a timeline track: no element handles an event timeline, a media timeline is unavailable, or an event timeline can't be subscribed or went stale. | Nothing, unless you need that timeline: then check the catalog and the publisher. |
| `VARIABLE_SKIPPED`       | The catalog uses a variable (MSF 5.4) that the URL fragment doesn't set, or sets with characters MSF 5.4.1 doesn't allow. The reference stays as written.           | Set the variable in the MSF URL's fragment.                                       |
| `INVALID_CATALOG_UPDATE` | The player rejected a catalog update and kept the last good catalog, or a switching set's GoP lengths don't line up for fast switching.                             | Fix the catalog the publisher sends.                                              |

`MEDIA_FAILURE` also arrives as a warning, when the LOCMAF source drops a group it can't rebuild.

## Reconnect after a lost session

When the session drops, the player reconnects by itself:

1. It moves to `reconnecting` and emits `reconnecting` with the attempt number, from 1.
2. It waits between attempts: 250 ms, doubling up to 5 s, each time ±20 %. While the page is hidden or offline it waits for it to come back, and that time doesn't count as an attempt.
3. Once a session is up again, it emits `client` with the new client, returns to the state it was in, and emits `reconnected`.
4. After `client.reconnectAttempts` failed attempts, 5 by default, it fails with `SESSION_LOST`.

A GOAWAY from the relay moves the session the same way, to the URI the GOAWAY names. The first connection, in `init()`, retries the same way, without the events.

Players on one page share the client, so they reconnect together. Elements and plugins that hold the client listen for `client` to pick up the new one.

`SESSION_LOST` is final for that player. To keep trying, destroy it and create a new one. With a `<video>` sink, wait for `destroy()` before the next player takes the element:

```ts
import { MOQtailPlayer, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
let player = create()

function create(): MOQtailPlayer {
  const next = new MOQtailPlayer({ plugins: [liveBuffer()] })
    .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
    .sink(video)
  next.on('error', (error) => {
    if (error.category === 'network') setTimeout(retry, 5000)
  })
  next.start().catch(() => {})
  return next
}

async function retry(): Promise<void> {
  await player.destroy()
  player = create()
}
```

## Handle a blocked autoplay

Browsers refuse to start playback with sound before the viewer interacts with the page. When `play()` or `start()` fails with a `NotAllowedError`, the player moves to `paused`, emits `autoplayblocked`, and rejects with that error. Show a play button and call `play()` from its click, as in the [example above](#listen-for-errors-and-warnings). A muted `<video>` autoplays.

On a `<canvas>`, video plays at once and audio waits for a user gesture on the page, without `autoplayblocked`.

## On the element

`<moqtail-video>` fires `error` like a `<video>`, and its `error` property holds a `MediaError` with the player error's message. The code comes from the category:

| Category  | `MediaError` code                |
| --------- | -------------------------------- |
| `aborted` | 1, `MEDIA_ERR_ABORTED`           |
| `network` | 2, `MEDIA_ERR_NETWORK`           |
| `media`   | 3, `MEDIA_ERR_DECODE`            |
| `source`  | 4, `MEDIA_ERR_SRC_NOT_SUPPORTED` |

```ts
import type { MOQtailVideoElement } from 'moqtail/player/component'

const video = document.querySelector<MOQtailVideoElement>('moqtail-video')!

video.addEventListener('error', () => {
  console.error(video.error?.code, video.error?.message)
})
video.player?.on('warning', (warning) => console.warn(warning.code, warning.message))
```

`video.player` is the player of the current source, for its states, warnings and reconnect events. Setting a new `src` creates a new player, so add your listeners again.

## More information

- [`PlayerError`](/ts/api/player/classes/PlayerError), [`PlayerErrorCode`](/ts/api/player/type-aliases/PlayerErrorCode) and [`PlayerErrorCategory`](/ts/api/player/type-aliases/PlayerErrorCategory)
- [`PlayerState`](/ts/api/player/type-aliases/PlayerState) and [`PlayerEvents`](/ts/api/player/interfaces/PlayerEvents)
- [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions), for `client.reconnectAttempts` and `catalogTimeoutMs`
- [`MOQtailVideoElement`](/ts/api/player/component/classes/MOQtailVideoElement)
