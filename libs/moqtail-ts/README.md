# moqtail

A TypeScript MOQT client and an MSF/CMSF player for the browser.

```bash
npm install moqtail
```

| Entry                      | What's in it                                      |
| -------------------------- | ------------------------------------------------- |
| `moqtail`                  | Everything below except the player                |
| `moqtail/client`           | `MOQtailClient`, its options, events and results  |
| `moqtail/model`            | Control messages, objects, parameters, properties |
| `moqtail/util`             | Logger, `Emitter`, telemetry, clock helpers       |
| `moqtail/player`           | `MOQtailPlayer`, elements, plugins                |
| `moqtail/player/component` | The `<moqtail-video>` web component               |

It runs in Chrome and Safari 26.4+ (WebTransport). It follows MOQT draft-22 and the MSF and CMSF editor's drafts. The player plays CMAF; LOC, captions and language switching aren't played yet. [DeepWiki](https://deepwiki.com/moqtail/moqtail) has the full list and the details this page leaves out.

## Client

Listeners attached before `init()` see the SETUP exchange too.

```ts
import { MOQtailClient, FullTrackName, GroupOrder, LocationFilter, LocationFilterType, RequestError } from 'moqtail'

const client = new MOQtailClient({ url: 'moqt://relay.example.com' })
client.on('messagereceived', (msg) => console.log('←', msg))
client.on('sessionterminated', (reason) => console.log('session ended', reason))
await client.init()

const sub = await client.subscribe({
  fullTrackName: FullTrackName.tryNew('live/demo', 'video'),
  priority: 128,
  groupOrder: GroupOrder.Original,
  locationFilter: new LocationFilter(LocationFilterType.NextObject),
})
if (sub instanceof RequestError) throw new Error(sub.reasonPhrase.phrase)

sub.done.then((publishDone) => console.log('subscription over', publishDone?.statusCode))
const reader = sub.stream.getReader()
for (let read = await reader.read(); !read.done; read = await reader.read()) {
  console.log(read.value.location, read.value.payload?.byteLength)
}
```

`fetch` reads objects already published. Its result carries the FETCH_OK, with the end location and the track properties:

```ts
const res = await client.fetch({
  fullTrackName: FullTrackName.tryNew('live/demo', 'video'),
  priority: 128,
  groupOrder: GroupOrder.Original,
  locationFilter: new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 10n, 0n, 12n),
})
if (!(res instanceof RequestError)) {
  console.log('ends at', res.fetchOk.endLocation)
  const reader = res.stream.getReader()
  for (let read = await reader.read(); !read.done; read = await reader.read()) console.log(read.value.location)
}
```

Publishing goes through `client.publishNamespace` and `client.publish`. [`apps/client-js`](../../apps/client-js) is a complete publisher.

## `<moqtail-video>`

The mostly hands-off option. It's a `<video>` that takes a `moqt://` URL, builds the player chain itself, and installs the default plugins: the buffer controller (`liveBuffer()`) is in, ABR is coming soon. Listing plugins in `config.plugins` replaces it, so leave `liveBuffer()` out of the list to play without it.

It needs two optional peer dependencies: `npm install custom-media-element media-tracks`.

```ts
import { defineMOQtailVideo } from 'moqtail/player/component'

defineMOQtailVideo()
```

```html
<moqtail-video src="moqt://relay.moqtail.dev#msf:moqtail-ch00--catalog" autoplay muted playsinline></moqtail-video>
```

`video.config = { catalogTimeoutMs: 5000 }` passes player options, read on each load. The `switch-from` attribute switches tracks with SWITCH_FROM instead of a new subscription. [`apps/client-js`](../../apps/client-js/src/lib/videojs.ts) wraps the element in a Video.js v10 skin.

## Player API

For more control. `MOQtailPlayer` installs no plugins, so list the buffer controller yourself.

```ts
import { MOQtailPlayer, PlayerError, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({ plugins: [liveBuffer({ catchupRate: 1.2 })] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog')
  .sink(video)

player.on('statechange', (state) => console.log(state))
player.on('error', (error: PlayerError) => console.error(error.code, error.category, error.message))

await player.init()
await player.play()
await player.seek(30)
```

A source is a catalog or a single track. `source()` throws `PlayerError` for a URL it can't use, such as another origin. Its element is picked in `init()`, once the relay's SETUP says which plugins are active, so a track no element accepts rejects `init()`. Everything later arrives on `error` (fatal) or `warning`. `player.destroy()` tears the chain down.

## Custom Elements And Plugins

The player is a chain of elements joined by pads. A source the probe picks for a track can be your own:

```ts
import { CapabilityCode, MOQtailPlayer, SourceElement, defineElement } from 'moqtail/player'

class LocSource extends SourceElement {
  // createPad('src', capabilities), write packets to it, then emit 'nomorepads'
}

const locSource = defineElement({
  type: 'source',
  name: 'loc-source',
  capabilities: [[{ direction: 'out', code: CapabilityCode.PACKAGING, value: 'loc' }]],
  probe: (track) => track.packaging === 'loc',
  init: async (context) => new LocSource(context),
})

new MOQtailPlayer().registerElement(locSource)
```

Among the probes that accept a track, the lowest `priority` wins (default 100), then the earlier definition: yours, then plugins' in list order, then built-in ones.

A plugin offers features (elements, switching set sources, a buffer controller) and the setup parameters they need. It can check the peer's SETUP: a plugin it rejects contributes nothing. `ssts()` (sender-side track switching) and `liveBuffer()` are built in:

```ts
import { MOQtailPlayer, definePlugin, liveBuffer, ssts } from 'moqtail/player'

const loc = definePlugin({
  name: 'loc',
  features: () => [{ type: 'element', item: locSource }],
})

new MOQtailPlayer({ plugins: [ssts({ algorithms: [0xff01n, 0n] }), loc(), liveBuffer()] })
```

A plugin's `init(config)` runs once per player, and its config type is what the factory takes.

DeepWiki explains the element and pad model, negotiation, and the interfaces elements can provide.
