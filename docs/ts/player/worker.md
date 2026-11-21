---
description: Decode and draw in the player's worker, keep everything on the main thread, or run your own element's engine in the worker.
---

# Run the player in a worker

By default the player starts a worker of its own and runs its heaviest work there, with no code from you. Use this page to turn that off, to follow the canvas rule it brings, or to run your own element's work in the worker too.

## What runs in the worker

With `preferWorker` (default `true`), the player starts its worker during `init()`. Elements that allow it run their engine there:

| Element                      | Runs in the worker when                                                                                                                          |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `webcodecs`, the LOC decoder | The worker has the decoder for the pad: `VideoDecoder` for video, `AudioDecoder` for audio.                                                      |
| `playout`, the canvas sink   | The worker has `OffscreenCanvas` and `requestAnimationFrame`, every pad it plays comes from the worker, and the canvas can still be handed over. |

So on a `<canvas>`, frames are decoded and drawn in the worker and never reach the main thread. The player's API is the same either way.

A `<video>` sink plays through Media Source Extensions on the main thread, so nothing runs in the worker, and the player stops the worker once `init()` is done. `<moqtail-video>` always plays on a `<video>`.

## Keep everything on the main thread

Set `preferWorker` to `false`:

```ts
import { MOQtailPlayer } from 'moqtail/player'

const player = new MOQtailPlayer({ preferWorker: false })
```

The player then starts no worker, and ignores the `worker` option.

## When the worker can't start

The player plays on the main thread instead, without an error, when:

- the browser has no `Worker`, or creating one throws;
- the worker script fails to load, for example because the page's Content Security Policy doesn't allow it;
- the worker doesn't answer within 10 s.

The default worker is a module script inside the package, which the player finds with `new URL(…, import.meta.url)`. [Install the library](/ts/install) shows how to let your bundler emit it, and which `worker-src` a Content Security Policy needs.

A worker that fails after it started fails the player with `MEDIA_FAILURE`.

## Give each player a new canvas

To draw from the worker, the canvas sink hands the canvas's control to it with `transferControlToOffscreen()`, which can't be undone. A later player on the same canvas fails with `UNSUPPORTED_SOURCE`. Create a new `<canvas>` for each player:

```ts
import { MOQtailPlayer, liveBuffer } from 'moqtail/player'

const stage = document.querySelector<HTMLDivElement>('#stage')!
let player: MOQtailPlayer | undefined

async function play(url: string): Promise<void> {
  await player?.destroy()
  const canvas = document.createElement('canvas')
  stage.replaceChildren(canvas)
  player = new MOQtailPlayer({ plugins: [liveBuffer()] }).source(url).sink(canvas)
  await player.start()
}

await play('moqt://relay.moqtail.dev#msf:moqtail-testsrc--catalog')
```

## Run your own engine in the worker

An element can run its work on each item, its engine, in the player's worker. The worker only has the code your bundler builds into it, so you write the worker's entry script and pass it to the player. The steps below split the grayscale transform from [Write plugins and elements](/ts/player/plugins#write-a-transform) into an engine and an element.

1. Write the engine. It implements `Engine`: `receive()` gets each item and `event()` each pad event, in order, and both write to the outputs the engine was given. Keep it in its own file, `grayscale-engine.ts`, so the worker doesn't load the rest of your app:

   ```ts
   import type { PadEvent, PadItem, PadOutput } from 'moqtail/player'
   import type { Engine, EngineInit } from 'moqtail/player/worker'

   export class GrayscaleEngine implements Engine {
     readonly #write: (item: PadOutput) => Promise<void>
     readonly #canvas = new OffscreenCanvas(1, 1)
     readonly #draw = this.#canvas.getContext('2d')!

     constructor(init: EngineInit) {
       this.#write = init.outputs[0]!
     }

     async receive(item: PadItem): Promise<void> {
       if (item.type !== 'frame' || !(item.frame.payload instanceof VideoFrame)) {
         await this.#write(item)
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
       await this.#write({ type: 'frame', frame: { ...item.frame, payload: gray } })
     }

     async event(event: PadEvent): Promise<void> {
       await this.#write({ type: 'event', event })
     }
   }
   ```

2. Serve it from the worker's entry script, `player.worker.ts`. `serveWorker()` serves the built-in engines too. The key is the name the element creates the engine by:

   ```ts
   import { serveWorker } from 'moqtail/player/worker'
   import { GrayscaleEngine } from './grayscale-engine'

   serveWorker({ grayscale: GrayscaleEngine })
   ```

   A name with whitespace, or a built-in engine's name (`webcodecs`, `playout`), throws a `TypeError`.

3. Write the element, `grayscale.ts`. It creates the engine in the worker when it can, and constructs it itself otherwise:

   ```ts
   import {
     CapabilityCode,
     TransformElement,
     defineElement,
     type Capability,
     type ElementContext,
     type Pad,
     type PadItem,
   } from 'moqtail/player'
   import type { Engine } from 'moqtail/player/worker'
   import { GrayscaleEngine } from './grayscale-engine'

   const RAW_VIDEO: Capability[] = [
     { direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' },
     { direction: 'out', code: CapabilityCode.ROLE, value: 'video' },
   ]

   class GrayscaleTransform extends TransformElement {
     #engine: Engine | undefined
     #output: Pad | undefined

     constructor(context: ElementContext) {
       super(context)
       this.on('padadded', (pad) => {
         if (pad.direction === 'sink') this.#start(pad)
       })
       this.on('padremoved', (pad, failed) => {
         if (pad.direction === 'sink' && this.#output) this.removePad(this.#output, failed)
       })
     }

     negotiate(pad: Pad): boolean {
       // One video pad per instance
       return !this.pads.some((p) => p.direction === 'sink') && this.matchesCapabilities(pad)
     }

     // Only items that reach the main thread arrive here: those of a pad not linked in the worker
     async receive(_pad: Pad, item: PadItem): Promise<void> {
       await this.#engine?.receive(item, 'video')
     }

     #start(input: Pad): void {
       const worker = this.context.worker
       if (worker) {
         const output = this.createPad('src', [
           ...RAW_VIDEO,
           { direction: 'out', code: CapabilityCode.THREAD, value: 'worker' },
         ])
         const engine = worker.create<GrayscaleEngine>('grayscale', { outputs: [output] })
         if (worker.inWorker(input)) worker.link(input, engine)
         this.#engine = engine
         this.#output = output
       } else {
         const output = this.createPad('src', RAW_VIDEO)
         this.#engine = new GrayscaleEngine({
           outputs: [(item) => output.write(item)],
           options: undefined,
           report() {},
         })
         this.#output = output
       }
       input.onEvent((event) => {
         void this.#engine?.event(event, 'video')
         return true
       })
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
     threads: ({ worker }) => (worker?.has('element:grayscale') ? 'both' : 'main'),
     init: async (context) => new GrayscaleTransform(context),
   })
   ```

   - `threads` says where the element's engines may run. `both` gives the element `context.worker` while the player has a worker; the check for `element:grayscale` makes sure that worker serves the engine. `main`, the default, gives it none. `worker` would make the element fail with `UNSUPPORTED_SOURCE` when there is no worker.
   - `context.worker.create(name, { outputs })` makes the engine in the worker. The outputs are the element's source pads, marked with `CapabilityCode.THREAD` `worker`, so the next element can take their items inside the worker too.
   - `link(input, engine)` sends the items of an input pad that comes from the worker to the engine, inside the worker. Items from the main thread arrive in `receive()`, which forwards them. Each worker pad the element takes needs `link()`, or `bridge()` to have them sent to `receive()` instead.
   - Without a worker, `context.worker` is undefined and the element runs the same engine class itself.

4. Pass the worker to the player with the `worker` option:

   ```ts
   import { MOQtailPlayer, liveBuffer } from 'moqtail/player'
   import { grayscale } from './grayscale'

   const canvas = document.querySelector('canvas')!
   const player = new MOQtailPlayer({
     plugins: [liveBuffer()],
     worker: () => new Worker(new URL('./player.worker.ts', import.meta.url), { type: 'module' }),
   })
     .source('moqt://relay.moqtail.dev#msf:moqtail-testsrc--catalog')
     .transform(grayscale)
     .sink(canvas)

   await player.start()
   ```

The decoder, your engine and the canvas sink now all run in the worker. An element that knows nothing about workers still works next to them: a worker pad it takes comes to it on the main thread.

## More information

- [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions), for `preferWorker` and `worker`
- [`serveWorker`](/ts/api/player/worker/functions/serveWorker), [`Engine`](/ts/api/player/worker/interfaces/Engine), [`EngineInit`](/ts/api/player/worker/interfaces/EngineInit) and [`EngineClass`](/ts/api/player/worker/type-aliases/EngineClass)
- [`ElementDefinition`](/ts/api/player/interfaces/ElementDefinition), for `threads`, and [`ElementContext`](/ts/api/player/interfaces/ElementContext)
- [`PlayoutOptions`](/ts/api/player/interfaces/PlayoutOptions)
- [How the player works](/guide/concepts/player)
