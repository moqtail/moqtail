# moqtail

A TypeScript MOQT client and an MSF/CMSF player for the browser.

```bash
npm install moqtail
```

`<moqtail-video>` also needs two optional peer dependencies: `npm install custom-media-element media-tracks`.

It runs in Chrome and Safari 26.4+ (WebTransport). It follows the latest MOQT interop target and the MSF and CMSF editor drafts ([supported drafts](https://docs.moqtail.dev/guide/)). The player plays CMAF and LOCMAF on a `<video>`, and LOC on a `<canvas>`; captions and language switching aren't played yet.

| Entry                      | What's in it                                      |
| -------------------------- | ------------------------------------------------- |
| `moqtail`                  | Everything below except the player                |
| `moqtail/client`           | `MOQtailClient`, its options, events and results  |
| `moqtail/model`            | Control messages, objects, parameters, properties |
| `moqtail/util`             | Logger, `Emitter`, telemetry, clock helpers       |
| `moqtail/player`           | `MOQtailPlayer`, elements, plugins                |
| `moqtail/player/component` | The `<moqtail-video>` web component               |
| `moqtail/player/worker`    | `serveWorker`, for a worker entry of your own     |

## `<moqtail-video>`

A `<video>` that takes a `moqt://` URL and installs the default buffer (`liveBuffer()`) and ABR (`abr()`) plugins.

```ts
import { defineMOQtailVideo } from 'moqtail/player/component'

defineMOQtailVideo()
```

```html
<moqtail-video src="moqt://relay.moqtail.dev#msf:moqtail-ch00--catalog" autoplay muted playsinline></moqtail-video>
```

## Player

For more control. `MOQtailPlayer` installs no plugins, so list the buffer controller yourself.

```ts
import { MOQtailPlayer, PlayerError, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({ plugins: [liveBuffer({ targetLatency: 2 })] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog')
  .sink(video)

player.on('statechange', (state) => console.log(state))
player.on('error', (error: PlayerError) => console.error(error.code, error.category, error.message))

await player.init()
await player.play()
await player.seek(30)
```

## Client

The client is a MOQT session on its own: subscribe, fetch and publish. Each request resolves to its result or to a `RequestError`; the objects come as a `ReadableStream`.

```ts
import { MOQtailClient, FullTrackName, GroupOrder, LocationFilter, LocationFilterType, RequestError } from 'moqtail'

const client = new MOQtailClient({ url: 'moqt://relay.example.com' })
await client.init()

const sub = await client.subscribe({
  fullTrackName: FullTrackName.tryNew('live/demo', 'video'),
  priority: 128,
  groupOrder: GroupOrder.Original,
  locationFilter: new LocationFilter(LocationFilterType.NextObject),
})
if (sub instanceof RequestError) throw new Error(sub.reasonPhrase.phrase)

const reader = sub.stream.getReader()
for (let read = await reader.read(); !read.done; read = await reader.read()) {
  console.log(read.value.location, read.value.payload?.byteLength)
}
```

## Documentation

The full documentation is at [docs.moqtail.dev/ts](https://docs.moqtail.dev/ts/):

- [Install the library](https://docs.moqtail.dev/ts/install): peer dependencies, bundlers, workers and CSP
- [Use `<moqtail-video>`](https://docs.moqtail.dev/ts/player/element) and [use `MOQtailPlayer`](https://docs.moqtail.dev/ts/player/player)
- [Tune live latency](https://docs.moqtail.dev/ts/player/latency)
- [Connect to a relay](https://docs.moqtail.dev/ts/client/connect) with the client
- [How the player works](https://docs.moqtail.dev/guide/concepts/player): elements, pads, the worker and plugins
- [API reference](https://docs.moqtail.dev/ts/api/)
