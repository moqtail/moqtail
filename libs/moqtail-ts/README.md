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

It runs in Chrome and Safari 26.4+ (WebTransport). It follows MOQT draft-22 and the MSF and CMSF editor's drafts. The player plays CMAF and LOCMAF; LOC, captions and language switching aren't played yet. [DeepWiki](https://deepwiki.com/moqtail/moqtail) has the full list and the details this page leaves out.

## How It Fits Together

```
MOQtailPlayer   source('moqt://relay/app#msf:ns--catalog')   sink(<video>)
     │ init()
     ▼
MOQtailClient ── WebTransport ── relay       one session per origin and setup, shared by players
     │ SERVER_SETUP decides which plugins are active
     ▼
catalog ──► one source per switching set ──pads──► [transforms] ──► MSE sink ──► <video>
```

The client is a MOQT session and nothing more: you can use it on its own to subscribe, fetch and publish. The player sits on top of it. It turns a `moqt://` URL into a chain of elements: sources that request tracks and write media to pads, optional transforms, and sinks that play what they receive. Plugins add elements and controllers to that chain, together with the SETUP parameters they need.

## MOQT In Short

A client opens one session to a relay and exchanges SETUP, where both sides state the parameters and extensions they support. Media is published as tracks. A track is named by a namespace (a tuple of fields, such as `live/demo`) and a track name, together the full track name. A track is a sequence of groups, each a sequence of objects; a group usually starts at a key frame, so it's where a subscriber can join. A subscriber asks for a track with SUBSCRIBE (objects as they're published) or FETCH (objects already published), and the relay answers with an OK or an error. A publisher announces a namespace with PUBLISH_NAMESPACE, or pushes a track with PUBLISH.

## Client

Each request resolves to its result or to a `RequestError`; the objects come as a `ReadableStream`. Listeners attached before `init()` see the SETUP exchange too.

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

The client's events, attached with `client.on()`:

- **Session**: `goaway` (the session is winding down, or a request moves to a new one), `sessionterminated` (once, with the reason).
- **Tracing**: `messagesent` and `messagereceived` for control messages, `datasent` and `datareceived` for stream headers and objects, `dataevent` for stream open, object and close with sizes only, `datagramsent` and `datagramreceived`, and `orderinggap` when an ordered request stops waiting for missing objects.
- **Answering the peer**: `forwardstatechange` (a track you publish may now send, or must stop), `namespacepublished`, `peerpublish` (the peer pushed a track, with its stream), `peerpublishdone`, `peerpublishskipped`, `peersubscribenamespace`, `peersubscribetracks`, `peernamespace` and `peernamespacedone`.

## `<moqtail-video>`

The mostly hands-off option, built on the player described below. It's a `<video>` that takes a `moqt://` URL, builds the player chain itself, and installs the default plugins: the buffer controller (`liveBuffer()`) and the ABR controller (`abr()`). Listing plugins in `config.plugins` replaces them, so leave one out of the list to play without it.

It needs two optional peer dependencies: `npm install custom-media-element media-tracks`.

```ts
import { defineMOQtailVideo } from 'moqtail/player/component'

defineMOQtailVideo()
```

```html
<moqtail-video src="moqt://relay.moqtail.dev#msf:moqtail-ch00--catalog" autoplay muted playsinline></moqtail-video>
```

`video.config = { catalogTimeoutMs: 5000 }` passes player options, read on each load. The `switch-from` attribute switches tracks with SWITCH_FROM instead of a new subscription. It fires the usual media events plus `streamtypechange`, lists renditions in `videoRenditions` and `audioRenditions` (no `selected` rendition is automatic switching, which needs a source that provides `ABR`, such as `ssts()`; otherwise the active one shows selected), and `video.player` is the `MOQtailPlayer` behind it, for the events below. [`apps/client-js`](../../apps/client-js/src/lib/videojs.ts) wraps the element in a Video.js v10 skin.

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

### From `source()` To Playing

The builder calls go `registerElement()`, `source()`, `transform()`, `sink()`, all before `init()`; anything else is a `TypeError`. `source()` takes an MSF URL, `moqt://authority/path#msf:<namespace>--<track>`, for a catalog or a single track. It throws `PlayerError` right away for a URL it can't use, such as another origin than an earlier `source()`.

The constructor already ran each plugin's `init` and `setup`, so their SETUP parameters are known. `init()` then:

1. Connects. Players with the same origin and setup share one client and its reconnects.
2. Keeps the plugins the relay's SERVER_SETUP accepts. The others contribute nothing.
3. Picks an element for each `source()` (see [Element selection](#element-selection)). A track no element accepts rejects `init()` with `UNSUPPORTED_SOURCE`.
4. Starts the sources. The catalog element reads the catalog, picks an element for every switching set and timeline the same way, and loads it. A switching set goes through a plugin's switch source when one accepts it, such as SSTS.
5. Routes each new source pad by capability: to the first transform that accepts it, otherwise to every sink that does.
6. Once every top-level source says it has all its pads (`nomorepads`), tells the sinks, and resolves when they're ready. The state is then `ready`, or `ended` when there's nothing to play.

After that, `play()`, `pause()`, `seek()` and `setPlaybackRate()` go to the media sink, and the state follows its `<video>`: `idle`, `connecting`, `ready`, then `playing`, `paused`, `stalled` or `ended`. A lost session moves to `reconnecting` and back to where it was. `error` and `destroyed` are final. `player.destroy()` tears the chain down in any state.

### Player Events

Attached with `player.on()`:

- **Playback**: `statechange` (new and previous state), `playing`, `pause`, `stalled`, `ended`, `autoplayblocked` (a `play()` the browser refused; the player is `paused`).
- **Session**: `client` (the connected client, again after every reconnect), `reconnecting` (with the attempt number), `reconnected`.
- **Content**: `catalog` (the catalog element, after the first catalog and each applied update), `renditionchange` (a source switched rendition), `selectionchange` (a source's selected renditions were set), `streamtypechange` (`live` or `on-demand`), `timelineevent` (records of the event timelines listed in `eventTimelines`).
- **Problems**: `error` is fatal and moves to `error`; `warning` isn't, such as a pad no element took (`PAD_UNLINKED`) or a catalog track that was skipped (`TRACK_SKIPPED`). Both carry a `PlayerError` with a `code`, a `category` (`source`, `network`, `media` or `aborted`) and the `element` it came from.

`player.mediaSources` are the sources playing media: `getRenditions()` lists their renditions, `switch(id)` changes one, and `selectRenditions(ids)` sets those ABR may choose from. `player.isAbrAvailable(source)` says whether a source switches automatically, by itself (`ABR`) or through an ABR controller.

## Element Selection

The player is a chain of elements joined by pads. Each element definition says what it is (`source`, `transform` or `sink`), the pads it creates (`out` capabilities) and the pads it accepts (`in` capabilities). A source definition also has a `probe` that says whether it handles a track.

For every track, the top-level one from the URL and every one the catalog lists, the candidates are, in order: the definitions passed to `registerElement()`, then the active plugins' elements (by plugin `priority`, then list order), then the built-in ones (catalog, CMAF, LOCMAF, media timeline, SAP timeline). Among the candidates whose probe accepts the track, the lowest `priority` wins (default 100), then the earlier one. So yours wins a tie with a plugin's, and both win a tie with a built-in one.

A source the probe picks can be your own:

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

Its pads still need a transform or sink whose `in` capabilities match them, or the player warns `PAD_UNLINKED`.

## Plugins

A plugin bundles features with the SETUP parameters they need, so a protocol extension or a controller is one entry in `plugins`. Its hooks are optional and run in this order:

1. `init(config)`, once per player in the constructor. Its config type is what the plugin's factory takes.
2. `setup(ctx, clientSetup)`, also in the constructor, adds SETUP parameters; returning false drops the plugin. Two plugins setting one parameter to different values throw `UNSUPPORTED_ORIGIN`.
3. `negotiated(ctx, serverSetup)`, after the relay's SERVER_SETUP; returning false leaves the plugin inactive, its features unused.
4. `features(ctx)` lists what it offers:
   - `element`: a candidate for [element selection](#element-selection).
   - `switch-source`: wraps the source of each media switching set it accepts, ranked like elements.
   - `buffer-controller`: only the first active one is used. The player attaches it while a negotiated stream is `live` and detaches it otherwise.
   - `abr-controller`: only the first active one is used. The player attaches it like the buffer controller, except while a media source provides `ABR` (switches itself, as `ssts()` sources do) or a rendition is picked manually (one of several selected).

`ssts()` (sender-side track switching), `liveBuffer()` and `abr()` are built in. `liveBuffer()` and `abr()` have priority `Infinity`, so any other buffer or ABR controller wins over them. For now `abr()` keeps the current quality.

```ts
import { MOQtailPlayer, definePlugin, liveBuffer, ssts } from 'moqtail/player'

const loc = definePlugin({
  name: 'loc',
  features: () => [{ type: 'element', item: locSource }],
})

new MOQtailPlayer({ plugins: [ssts({ algorithms: [0xff01n, 0n] }), loc(), liveBuffer()] })
```

Plugin code reaches the player through what it's handed. An element's `context.player` has the player's events and options, and `context.client` is the current client; it changes after a reconnect, so listen again on the player's `client` event. A buffer controller gets the player and the media sink in `attach()`:

```ts
import { definePlugin, type BufferController, type MOQtailPlayer } from 'moqtail/player'

class StallLogger implements BufferController {
  #player: MOQtailPlayer | undefined
  readonly #onStalled = () => console.log('stalled')

  attach(player: MOQtailPlayer): void {
    this.#player = player
    player.on('stalled', this.#onStalled)
  }

  detach(): void {
    this.#player?.off('stalled', this.#onStalled)
  }
}

const stallLogger = definePlugin({
  name: 'stall-logger',
  features: () => [{ type: 'buffer-controller', item: new StallLogger() }],
})
```

DeepWiki explains the pad model, negotiation, the catalog, each built-in element, and the interfaces elements can provide.
