---
description: Extend the player with plugins that add controllers and setup parameters, and with your own source, transform and sink elements.
---

# Write plugins and elements

Use this page to extend the player. A plugin adds controllers or elements to it, with the SETUP parameters they need. An element is one step of the chain from a track to the screen: a source, a transform or a sink. [How the player works](/guide/concepts/player) explains the chain they plug into.

## Write a plugin

`definePlugin()` checks a plugin definition and returns a factory. Calling the factory with the plugin's config gives an entry for the `plugins` option. Every hook is optional:

| Hook                           | Runs                                | Does                                                                                                        |
| ------------------------------ | ----------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `init(config)`                 | Once per player, in the constructor | Returns the plugin's context, `ctx`, which the other hooks get. Its parameter type is the factory's config. |
| `setup(ctx, clientSetup)`      | In the constructor, after `init`    | Adds SETUP parameters to `clientSetup`. Returning `false` drops the plugin.                                 |
| `negotiated(ctx, serverSetup)` | After the relay's SERVER_SETUP      | Returning `false` leaves the plugin inactive: none of its features are used.                                |
| `features(ctx)`                | Once per player, after `init`       | Lists what the plugin offers.                                                                               |

Without `setup` or `negotiated`, the plugin counts as accepted. A feature is one of these:

| `type`              | `item`                      | How the player uses it                                                                                                                        |
| ------------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `buffer-controller` | a `BufferController`        | Only the first active one. Attached while a negotiated stream is live.                                                                        |
| `abr-controller`    | an `AbrController`          | Only the first active one. Attached like the buffer controller, except while a media source switches itself or a rendition is picked by hand. |
| `element`           | an element definition       | A candidate source, transform or sink, as if passed to `registerElement()`.                                                                   |
| `switch-source`     | a source element definition | Loaded by the catalog for each media switching set whose first track it accepts, as `ssts()` does.                                            |

Active plugins are ranked by `priority`, lowest first (default 100), then by their order in `plugins`. `liveBuffer()` and `abr()` have priority `Infinity`, so a controller of yours wins over them.

### Offer an ABR controller

This plugin keeps the video at or below a height, in place of `abr()`. Its config is what `init` takes, `cap.ts`:

```ts
import { definePlugin, type AbrController, type MOQtailPlayer } from 'moqtail/player'

/** Keeps each video source on its best rendition at or below `maxHeight`. */
class HeightCap implements AbrController {
  readonly #maxHeight: number
  readonly #pick = (): void => {
    for (const source of this.#player?.mediaSources ?? []) {
      const best = source
        .getRenditions()
        .filter((r) => r.selected && r.height !== undefined && r.height <= this.#maxHeight)
        .sort((a, b) => b.bitrate - a.bitrate)[0]
      if (best && !best.active) source.switch(best.id).catch(() => {})
    }
  }
  #player: MOQtailPlayer | undefined

  constructor(maxHeight: number) {
    this.#maxHeight = maxHeight
  }

  attach(player: MOQtailPlayer): void {
    this.#player = player
    player.on('catalog', this.#pick)
    this.#pick()
  }

  detach(): void {
    this.#player?.off('catalog', this.#pick)
    this.#player = undefined
  }
}

export const heightCap = definePlugin({
  name: 'height-cap',
  init: ({ maxHeight }: { maxHeight: number }) => {
    if (!(maxHeight > 0)) throw new TypeError('maxHeight must be a positive number')
    return new HeightCap(maxHeight)
  },
  features: (controller) => [{ type: 'abr-controller', item: controller }],
})
```

List it in `plugins`:

```ts
import { MOQtailPlayer, liveBuffer } from 'moqtail/player'
import { heightCap } from './cap'

const video = document.querySelector('video')!
const player = new MOQtailPlayer({ plugins: [liveBuffer(), heightCap({ maxHeight: 720 })] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

await player.start()
```

A controller gets the player and the media sink in `attach()`, and undoes everything in `detach()`. A `TypeError` thrown by `init` reaches the caller of the `MOQtailPlayer` constructor. The player throws one itself when two entries in `plugins` have the same `name`.

### Add a SETUP parameter

A protocol extension sends a SETUP parameter, and turns its features on only when the relay answers with it:

```ts
import { KeyValuePair } from 'moqtail/model'
import { definePlugin } from 'moqtail/player'
import { textSource } from './text-source'

/** A setup option the relay echoes when it supports the extension. */
const TEXT_EXTENSION = 0x7f00n

export const textTracks = definePlugin({
  name: 'text-tracks',
  setup: (_, clientSetup) => {
    clientSetup.extensions = [KeyValuePair.tryNewVarInt(TEXT_EXTENSION, 1)]
    return true
  },
  negotiated: (_, serverSetup) =>
    serverSetup.options.extensions?.some((option) => option.typeValue === TEXT_EXTENSION) ?? false,
  features: () => [{ type: 'element', item: textSource }],
})
```

`textSource` is the source from [Register a source](#register-a-source). The parameters of every plugin are sent, active or not. Two plugins that set one parameter to different values make the constructor throw `UNSUPPORTED_ORIGIN`. Players on one page share a client, so they need the same parameters too.

## Write an element

`defineElement()` checks an element definition and returns it with its types. An invalid definition throws a `TypeError`. A definition has these fields:

| Field                    | What it is                                                                                                                                                                                 |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `type`                   | `'source'`, `'transform'` or `'sink'`.                                                                                                                                                     |
| `name`                   | Not empty, no whitespace. Errors and warnings name the element by it.                                                                                                                      |
| `capabilities`           | Capability bundles: each `in` bundle describes a pad the element takes, each `out` bundle a pad it makes.                                                                                  |
| `probe(track)`           | Sources only, and required for them: whether the element handles the track.                                                                                                                |
| `priority`               | Among the probes that accept a track, the lowest wins. Default 100.                                                                                                                        |
| `provides`               | The [interfaces](#element-interfaces) the element implements.                                                                                                                              |
| `supports(track)`        | Whether the element can handle the track's media, asked before any instance exists. Leaving it out means yes.                                                                              |
| `threads(environment)`   | Where the element's engines run: `main`, the default, `both` or `worker`. See [Run the player in a worker](/ts/player/worker#run-your-own-engine-in-the-worker).                           |
| `init(context, options)` | Creates the instance. Reject with a `PlayerError` to set the code, for example when a browser API is missing. Any other rejection of an element you declared becomes `UNSUPPORTED_SOURCE`. |

To fetch an element's code only when a stream needs it, keep every field but `init` in a module of its own, and register `lazyElement(meta, load)`. `load` returns the full definition, from a module the player imports on the first `init`:

```ts
import { lazyElement } from 'moqtail/player'
import { TEXT_META } from './text-meta'

const TextSource = lazyElement(TEXT_META, () => import('./text-source').then((m) => m.TextSource))
```

Here `./text-source` exports `defineElement({ ...TEXT_META, init })`. Your bundler splits it into a chunk of its own, as long as nothing else imports it statically.

The instance extends one of three classes:

- `SourceElement` creates its pads with `createPad('src', capabilities)`, writes media to them, and emits `nomorepads` once every pad it will make exists.
- `TransformElement` says in `negotiate(pad)` whether it takes an upstream pad, gets the pad's items in `receive(pad, item)`, and writes its output to source pads it creates.
- `SinkElement` negotiates and receives like a transform, and emits `ready` once it can play, after the player passes on `nomorepads`.

### Pads and capabilities

A capability is a `{ direction, code, value }`. The `code` is a `CapabilityCode`, such as `PACKAGING` (`cmaf`, `loc`, `raw`) or `ROLE` (`video`, `audio`). In a bundle, capabilities with the same code are alternatives, and different codes must all hold.

The player routes each new source pad to the first transform after its element that takes it, or else to every sink that does. A pad that none of them takes gets the fewest transforms that make it acceptable, such as the built-in `webcodecs` decoder in front of a canvas. If there are none, the player warns `PAD_UNLINKED`.

A pad carries items in order:

- `packet`: the payload of one object.
- `frame`: a decoded `VideoFrame` or `AudioData`. The element that receives it owns it, and closes it or passes it on.
- `properties`: pad properties, such as `mimeType` or `initData`.
- `event`: a `discontinuity` or the end of the stream, `eos`.

`await pad.write(item)` applies backpressure: it waits while a connection holds 32 items. Events and signals an element doesn't handle pass through it to its other pads.

### Element interfaces

What the player asks of an element beyond its pads goes through the interfaces it lists in `provides`. TypeScript checks the list against what `init` returns.

| Key            | Interface            | The player uses it for                                                                                                                                               |
| -------------- | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MEDIA_SOURCE` | `MediaSourceElement` | Renditions: `getRenditions()`, `switch()`, `selectRenditions()`, `setTracks()`. `player.mediaSources` lists these sources, except those another media source loaded. |
| `ABR`          | `MediaSourceElement` | A media source that switches among its selected renditions itself. The ABR controller leaves it alone.                                                               |
| `MEDIA_SINK`   | `MediaSinkElement`   | Playback: `play()`, `pause()`, `seek()`, `setPlaybackRate()`, the time and the media events. The first one is `player.mediaSink`.                                    |

`hasInterface(element, MEDIA_SINK)` narrows an element to the interface, when its definition lists it.

### What an element can reach

Each instance gets an `ElementContext` in `init`, kept as `this.context`:

- `player`: the player's events and options, and its buffer controller.
- `client`: the current client. It changes after a reconnect, so listen for the player's `client` event.
- `signal`: aborted when the instance is destroyed.
- `tracks` and `catalog`: the catalog tracks the element was loaded for, and the catalog.
- `fail(error)`: reports a fatal error from outside `receive()`, as a `PlayerError` with the element's name in `element`.

## Write a transform

This transform turns video gray. It takes `raw` video pads, the decoded frames a canvas plays, and makes one output pad for its one input, `grayscale.ts`:

```ts
import {
  CapabilityCode,
  TransformElement,
  defineElement,
  type ElementContext,
  type Pad,
  type PadItem,
} from 'moqtail/player'

class GrayscaleTransform extends TransformElement {
  readonly #canvas = new OffscreenCanvas(1, 1)
  readonly #draw = this.#canvas.getContext('2d')!
  #output: Pad | undefined

  constructor(context: ElementContext) {
    super(context)
    this.on('padadded', (pad) => {
      if (pad.direction !== 'sink') return
      this.#output = this.createPad('src', [
        { direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' },
        { direction: 'out', code: CapabilityCode.ROLE, value: 'video' },
      ])
    })
    this.on('padremoved', (pad, failed) => {
      if (pad.direction === 'sink' && this.#output) this.removePad(this.#output, failed)
    })
  }

  negotiate(pad: Pad): boolean {
    // One video pad per instance
    return !this.pads.some((p) => p.direction === 'sink') && this.matchesCapabilities(pad)
  }

  async receive(_pad: Pad, item: PadItem): Promise<void> {
    if (item.type !== 'frame' || !(item.frame.payload instanceof VideoFrame)) {
      await this.#output?.write(item)
      return
    }
    const frame = item.frame.payload
    if (this.#canvas.width !== frame.displayWidth || this.#canvas.height !== frame.displayHeight) {
      this.#canvas.width = frame.displayWidth
      this.#canvas.height = frame.displayHeight
    }
    this.#draw.filter = 'grayscale(1)'
    this.#draw.drawImage(frame, 0, 0)
    const gray = new VideoFrame(this.#canvas, { timestamp: frame.timestamp })
    frame.close()
    await this.#output?.write({ type: 'frame', frame: { ...item.frame, payload: gray } })
  }
}

export const grayscale = defineElement({
  type: 'transform',
  name: 'grayscale',
  capabilities: [
    [
      { direction: 'in', code: CapabilityCode.PACKAGING, value: 'raw' },
      { direction: 'in', code: CapabilityCode.ROLE, value: 'video' },
    ],
    [{ direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' }],
  ],
  init: async (context) => new GrayscaleTransform(context),
})
```

- `matchesCapabilities(pad)` checks the pad against the definition's `in` bundles.
- Items other than video frames, such as pad properties, go on unchanged.
- Events need no code: unhandled, they pass on to the output pad. A removed input removes the output, keeping `failed`.

Add it to the chain with `transform()`, between `source()` and `sink()`:

```ts
import { MOQtailPlayer, liveBuffer } from 'moqtail/player'
import { grayscale } from './grayscale'

const canvas = document.querySelector('canvas')!
const player = new MOQtailPlayer({ plugins: [liveBuffer()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-testsrc--catalog')
  .transform(grayscale)
  .sink(canvas)

await player.start()
```

The chain loads the `webcodecs` decoder in front of the transform, so it receives decoded frames. The transform runs on the main thread: [Run the player in a worker](/ts/player/worker#run-your-own-engine-in-the-worker) moves it into the worker.

`transform()` adds one instance to the chain, which is offered every pad that reaches it. `registerElement()` instead makes a transform a candidate that the chain loads only where a pad needs it to reach a sink, one instance per pad. A canvas already takes `raw` video, so the chain would never load the grayscale transform that way.

## Register a source

The player picks the source element for every track by probe: the one from the URL, and each one the catalog lists. A source you register wins a tie with the built-in ones:

```ts
import { CapabilityCode, MOQtailPlayer, SourceElement, defineElement } from 'moqtail/player'

class TextSource extends SourceElement {
  // Creates its pads, writes each object to them as a packet, then emits 'nomorepads'
}

export const textSource = defineElement({
  type: 'source',
  name: 'text',
  capabilities: [[{ direction: 'out', code: CapabilityCode.PACKAGING, value: 'text' }]],
  probe: (track) => track.packaging === 'text',
  init: async (context) => new TextSource(context),
})

export const player = new MOQtailPlayer().registerElement(textSource)
```

Call `registerElement()` before `source()`. For each track, the candidates come in this order:

1. The definitions passed to `registerElement()`.
2. The `element` features of the active plugins, by plugin priority, then order.
3. The built-in sources: catalog, CMAF, LOCMAF, LOC, media timeline and SAP timeline.

Among those whose `probe` accepts the track, the lowest `priority` wins, then the earliest. Transforms the chain loads come from the same three places, with the built-in `webcodecs` last.

A track URL that no probe accepts alone is requested, and probed again with its `trackProperties`, then after each object it reads with `objects`: see [Which tracks play](/ts/reference/msf-url#which-tracks-play). The source's options then carry the identifiers it was accepted with, as `source`. The built-in media sources play on the request the player sent, which a source of your own can't read: the player cancels it after `init()`.

The pads of the source still need a transform or sink whose `in` capabilities match them, such as a sink you pass to `player.sink(definition, options)`. Without one, the player warns `PAD_UNLINKED`. [Subscribe and fetch](/ts/client/subscribe) shows how to read a track with `context.client`.

## More information

- [`definePlugin`](/ts/api/player/functions/definePlugin), [`PluginDefinition`](/ts/api/player/interfaces/PluginDefinition), [`Plugin`](/ts/api/player/interfaces/Plugin) and [`Feature`](/ts/api/player/type-aliases/Feature)
- [`BufferController`](/ts/api/player/interfaces/BufferController) and [`AbrController`](/ts/api/player/interfaces/AbrController)
- [`defineElement`](/ts/api/player/functions/defineElement), [`ElementDefinition`](/ts/api/player/interfaces/ElementDefinition) and [`ElementContext`](/ts/api/player/interfaces/ElementContext)
- [`SourceElement`](/ts/api/player/classes/SourceElement), [`TransformElement`](/ts/api/player/classes/TransformElement), [`SinkElement`](/ts/api/player/classes/SinkElement) and [`Pad`](/ts/api/player/classes/Pad)
- [`CapabilityCode`](/ts/api/player/enumerations/CapabilityCode), [`PadItem`](/ts/api/player/type-aliases/PadItem) and [`PadEvent`](/ts/api/player/type-aliases/PadEvent)
- [`MediaSourceElement`](/ts/api/player/interfaces/MediaSourceElement), [`MediaSinkElement`](/ts/api/player/interfaces/MediaSinkElement) and [`hasInterface`](/ts/api/player/functions/hasInterface)
- [`MOQtailPlayer`](/ts/api/player/classes/MOQtailPlayer), for `registerElement()`, `transform()` and `sink()`
