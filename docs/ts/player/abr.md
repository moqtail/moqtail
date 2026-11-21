---
description: Let the player switch video renditions as the bandwidth changes, pick a rendition by hand, or turn switching off.
---

# Control adaptive bitrate

Use this page to change how the player picks the video rendition of a live stream. The `abr()` plugin does the picking. `<moqtail-video>` installs it by default. `MOQtailPlayer` installs no plugins, so you list it yourself:

```ts
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({ plugins: [liveBuffer(), abr()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

await player.start()
```

## What it decides

`abr()` picks the rendition of the video source of a live stream. The player attaches it once the stream is live, and detaches it while:

- the stream is on demand, which plays the first rendition of each set;
- a media source switches renditions itself, as the sources of [`ssts()`](#let-the-relay-switch-with-ssts) do;
- a rendition is picked by hand.

While attached, it does nothing while the viewer has paused, or while fewer than two renditions are selected or one has no known bitrate.

A live stream starts on its middle rendition, so ABR can move either way. Every 500 ms a small neural network chooses one of four actions, among those that rules allow on that tick:

- **Down** only on trouble: a stall, frames arriving at less than 0.8 of their frame rate, or less than 0.5 s of buffer. It goes straight to the highest rendition within 0.8 of the measured throughput.
- **Up** only when recent probes found room for the next rendition, with a 0.9 margin. It goes straight to the highest rendition that fits.
- **Probe** fetches objects it already received again, at a priority below live media, so the relay sends them only into spare capacity. That is how it measures room above the current bitrate.
- **Stay** is the only action while a switch settles, and for 1 s after.

For its first 2 s of live video it stays put unless playback stalls. [How the player works](/guide/concepts/player) explains why it probes and what the model sees.

## Options

`abr()` takes these options:

| Option       | Default         | Effect                                                                                                      |
| ------------ | --------------- | ----------------------------------------------------------------------------------------------------------- |
| `tickMs`     | `500`           | Milliseconds between decisions. Above 0 and finite. The bundled model was trained at 500.                   |
| `model`      | the bundled one | A `ModelBlob` to use instead. A model trained on another feature list is rejected, and ABR then stays idle. |
| `onDecision` | none            | Called with every decision, for logging.                                                                    |

An invalid `tickMs` is a `TypeError` when the player is created.

To log each decision with the name of its action:

```ts
import { Action, MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const names = Object.fromEntries(Object.entries(Action).map(([name, value]) => [value, name]))

const player = new MOQtailPlayer({
  plugins: [
    liveBuffer(),
    abr({
      onDecision: ({ action, observation }) => {
        console.log(names[action], { buffer: observation.forwardBuffer, throughput: observation.throughput })
      },
    }),
  ],
})
```

A decision also carries `switchedTo`, the index in `observation.ladder` it switched to, and `probeResult`, on the tick a probe ended.

## Follow rendition changes

The player emits `renditionchange` when a source switched rendition, and `selectionchange` when its selected renditions were set:

```ts
player.on('renditionchange', (_source, rendition) => {
  console.log(`now playing ${rendition.id}: ${rendition.width}x${rendition.height}, ${rendition.bitrate} b/s`)
})
player.on('selectionchange', (source) => {
  const selected = source.getRenditions().filter((r) => r.selected)
  console.log(`ABR picks from ${selected.map((r) => r.id).join(', ')}`)
})
```

`renditionchange` doesn't fire for the rendition a source starts with. Read `active` from `getRenditions()` for that one.

## Pick a rendition by hand

After `init()`, each source in `player.mediaSources` lists its renditions with `getRenditions()`. `selectRenditions(ids)` sets the renditions ABR may choose from:

- One id picks that rendition, and the player detaches the ABR controller.
- Several ids keep ABR, choosing among them.
- Every id gives automatic switching back.

When the playing rendition isn't selected anymore, the source switches to the selected one closest in bitrate.

```ts
import { CapabilityCode } from 'moqtail/player'

const videoSource = player.mediaSources.find((source) =>
  source.pads.some((pad) => pad.capabilities.some((c) => c.code === CapabilityCode.ROLE && c.value === 'video')),
)
if (videoSource) {
  const renditions = videoSource.getRenditions()
  const lowest = renditions.reduce((a, b) => (b.bitrate < a.bitrate ? b : a))

  // Pick one rendition: the ABR controller lets go
  await videoSource.selectRenditions([lowest.id])

  // Let ABR choose among the renditions up to 720p
  await videoSource.selectRenditions(renditions.filter((r) => (r.height ?? 0) <= 720).map((r) => r.id))

  // Back to automatic switching over every rendition
  await videoSource.selectRenditions(renditions.map((r) => r.id))
}
```

`switch(id)` changes the rendition too, but leaves the selection as it is, so ABR may switch away again. `player.isAbrAvailable(source)` says whether a source switches automatically.

On `<moqtail-video>`, the video renditions are in `videoRenditions`. Selecting one picks it. No selected rendition means automatic switching. Where nothing switches automatically, as on demand, the playing rendition shows as selected.

```ts
import type { MOQtailVideoElement } from 'moqtail/player/component'

const video = document.querySelector<MOQtailVideoElement>('moqtail-video')!

video.videoRenditions.addEventListener('change', () => {
  console.log('selected rendition', video.videoRenditions.selectedIndex)
})

// Pick the first rendition
video.videoRenditions.selectedIndex = 0
// Back to automatic switching
video.videoRenditions.selectedIndex = -1
```

## Turn ABR off

With `MOQtailPlayer`, leave `abr()` out of `plugins`. On `<moqtail-video>`, set `config.plugins` to a list without it, before `src`. Keep `liveBuffer()` in it:

```ts
import { liveBuffer } from 'moqtail/player'
import type { MOQtailVideoElement } from 'moqtail/player/component'

const video = document.querySelector<MOQtailVideoElement>('moqtail-video')!
video.config = { plugins: [liveBuffer()] }
video.src = 'moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog'
```

A live stream then stays on the middle rendition it starts with, until you pick another.

## Let the relay switch with SSTS

With sender-side track switching (SSTS), the relay picks the rendition: the player subscribes to the renditions of a switching set, and the relay decides which one it forwards. SSTS follows a provisional draft, `draft-wilaw-moq-moqt-ssts`, and works on live streams only.

Add the `ssts()` plugin:

```ts
import { MOQtailPlayer, abr, liveBuffer, ssts } from 'moqtail/player'

const player = new MOQtailPlayer({ plugins: [ssts(), liveBuffer(), abr()] })
```

- The plugin offers its algorithms at setup, most preferred first: `SSTS_ALGORITHMS` (`0n` and `0xff01n`) by default, or your own with `ssts({ algorithms })`.
- It is active only when the relay accepts one of them. The MOQtail relay does when `ssts_enable` is true in its [configuration file](/relay/reference/config). Otherwise the player plays as if the plugin weren't listed, and `abr()` picks the rendition.
- SSTS sources switch themselves, so the player doesn't attach the `abr()` controller to them.
- Picking one rendition by hand pins the relay to it, as above.

On `<moqtail-video>`, set `config.plugins` to the same list.

## More information

- [`abr`](/ts/api/player/variables/abr), [`MlAbrConfig`](/ts/api/player/interfaces/MlAbrConfig), [`Decision`](/ts/api/player/interfaces/Decision) and [`Observation`](/ts/api/player/interfaces/Observation)
- [`MediaSourceElement`](/ts/api/player/interfaces/MediaSourceElement) and [`Rendition`](/ts/api/player/interfaces/Rendition)
- [`PlayerEvents`](/ts/api/player/interfaces/PlayerEvents)
- [`ssts`](/ts/api/player/variables/ssts) and [`SstsConfig`](/ts/api/player/interfaces/SstsConfig)
- [`AbrController`](/ts/api/player/interfaces/AbrController), to write your own: see [Write plugins and elements](/ts/player/plugins)
- [How the player works](/guide/concepts/player)
