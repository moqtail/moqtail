---
description: Register the <moqtail-video> element, give it an MSF URL and player options, and read its renditions, player and events.
---

# Use `<moqtail-video>`

Use `<moqtail-video>` to play a stream wherever a `<video>` fits. It takes an MSF URL in `src`, builds a `MOQtailPlayer` that plays on a `<video>` in its shadow root, and forwards the media element API, so code and player UIs written for a `<video>` work with it.

It needs the optional peer dependencies of `moqtail`:

```sh
npm install moqtail custom-media-element media-tracks
```

[Install the library](/ts/install) covers the bundler setting the player needs.

## Register the element

```ts
import { defineMOQtailVideo } from 'moqtail/player/component'

defineMOQtailVideo()
```

`defineMOQtailVideo()` registers the element as `moqtail-video`. Pass another name to register it under that name: `defineMOQtailVideo('live-video')`. It does nothing when the name is taken, or where there is no `customElements`, as in server-side rendering, so you can call it more than once.

To add behaviour to the element, extend `MOQtailVideoElement` and register your class with `customElements.define()`. The Video.js integration in [`apps/client-js/src/lib/videojs.ts`](https://github.com/moqtail/moqtail/blob/main/apps/client-js/src/lib/videojs.ts) registers `MediaAttachMixin(MOQtailVideoElement)` this way.

## Set the source

```html
<moqtail-video
  src="moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog"
  autoplay
  muted
  playsinline
  controls
></moqtail-video>
```

`src` is the MSF URL of a stream's catalog. Each time it changes, the element destroys its player and builds a new one. Removing it stops playback. `load()` rebuilds the player from the same `src`.

When the element loads a source:

- With `autoplay`, it plays at once. Browsers autoplay only muted media, so add `muted`.
- With `preload="none"`, it waits for `play()` before it connects.
- Otherwise it connects and reads the catalog, ready for `play()`.

The element reads these attributes itself:

| Attribute     | Effect                                                                                                                 |
| ------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `src`         | The MSF URL to play. A change reloads.                                                                                 |
| `switch-from` | Switches renditions with SWITCH_FROM, which moves the subscription to the new track, instead of a second subscription. |
| `autoplay`    | Plays as soon as the source loads.                                                                                     |
| `preload`     | `none` connects on `play()` only.                                                                                      |

`switch-from` is read on each load, so set it before `src`. The MOQtail relay supports SWITCH_FROM. The element passes the other standard media attributes to its inner `<video>`: `muted`, `controls`, `playsinline`, `poster`, `loop`, `crossorigin`, `controlslist`, `autopictureinpicture`, `disablepictureinpicture` and `disableremoteplayback`.

## Pass player options

`config` takes the [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions) of the player the element builds:

```ts
import type { MOQtailVideoElement } from 'moqtail/player/component'

const video = document.querySelector<MOQtailVideoElement>('moqtail-video')!
video.config = { catalogTimeoutMs: 5000 }
video.load()
```

The element reads `config` when it loads a source, so set it before `src`, or call `load()` after it, as above. Changing `config` alone doesn't restart playback. A `config` set on the element before `defineMOQtailVideo()` runs is kept.

### Choose the plugins

When `config.plugins` is unset or empty, the element installs two plugins: `liveBuffer()`, which holds the live latency, and `abr()`, which switches renditions. A list with plugins replaces both, so list the ones you want to keep:

```ts
import { abr, liveBuffer } from 'moqtail/player'
import type { MOQtailVideoElement } from 'moqtail/player/component'

const video = document.createElement('moqtail-video') as MOQtailVideoElement
video.config = { plugins: [liveBuffer({ targetLatency: 2 }), abr()] }
video.muted = true
video.autoplay = true
video.src = 'moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog'
document.body.append(video)
```

To play without ABR, list `liveBuffer()` alone. An empty list gets the two defaults.

## Pick a rendition

The element lists the renditions of the playing video and audio in `videoRenditions` and `audioRenditions`, as [media-tracks](https://github.com/muxinc/media-elements/tree/main/packages/media-tracks) defines them. They fill once the catalog is read. Each rendition has the `id` (the track name), `codec`, `bitrate`, and for video `width`, `height` and `frameRate`.

```ts
for (const rendition of video.videoRenditions) {
  console.log(rendition.id, `${rendition.width}x${rendition.height}`, rendition.bitrate)
}

// Play only the third rendition
video.videoRenditions.selectedIndex = 2
// Back to automatic switching
video.videoRenditions.selectedIndex = -1
```

No selected rendition means automatic switching. Where nothing switches automatically, as in a VOD stream, the playing rendition shows as selected. [Control adaptive bitrate](/ts/player/abr#pick-a-rendition-by-hand) has the details.

## Use the player behind it

`video.player` is the `MOQtailPlayer` for the current `src`. It is `undefined` without a `src`, or when the element could not build a player, which it reports with `error`. The element builds the player when `src` is set, so you can listen to it right after:

```ts
video.src = 'moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog'
video.player?.on('statechange', (state) => console.log(state))
video.player?.on('renditionchange', (_source, rendition) => console.log('now playing', rendition.id))
```

Every new `src` gets a new player, so add your listeners again after each one.

## Listen for events

The element fires the media events of its inner `<video>`, such as `playing`, `pause`, `waiting`, `timeupdate` and `ended`, and two of its own:

- `error`, when the player fails. `video.error` holds a `MediaError` with the player error's message. The inner `<video>`'s own errors are not fired: the player recovers from those, and reports the ones it can't.
- `streamtypechange`, when `video.streamType` changes to `live`, `on-demand` or back to `unknown`.

```ts
video.addEventListener('error', () => console.error(video.error?.code, video.error?.message))
video.addEventListener('streamtypechange', () => console.log(video.streamType))
```

[Handle states and errors](/ts/player/errors#on-the-element) maps the error categories to `MediaError` codes.

For live streams the element also has `liveEdgeStart` and `targetLiveWindow`, which player UIs such as Video.js read to treat the stream as live. When a live stream has a DVR window, `currentTime`, `seekable` and `buffered` count from the start of the window. Without one, setting `currentTime` does nothing.

## More information

- [`defineMOQtailVideo`](/ts/api/player/component/functions/defineMOQtailVideo) and [`MOQtailVideoElement`](/ts/api/player/component/classes/MOQtailVideoElement)
- [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions)
- [Use `MOQtailPlayer`](/ts/player/player), for what the element doesn't expose, such as a `<canvas>`
- [Tune live latency](/ts/player/latency) and [Control adaptive bitrate](/ts/player/abr)
