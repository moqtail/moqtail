---
description: Add the moqtail package to a web app, let your bundler emit the player's worker, and allow the player in a Content Security Policy.
---

# Install the library

Use this to add `moqtail` to a web app that you build with a bundler. The [library overview](/ts/) lists what each entry point holds.

## Install the package

```sh
npm install moqtail
```

The package is ESM only, with no CommonJS build, so load it with `import`. It ships its TypeScript types.

If you use `<moqtail-video>`, also install its two optional peer dependencies:

```sh
npm install custom-media-element media-tracks
```

Only `moqtail/player/component` imports them. The client and `MOQtailPlayer` work without them.

The player loads the code of the `<video>` and `<canvas>` sinks, the WebCodecs decoder and the LOCMAF source when a stream first needs it, from chunks your bundler emits next to your app. [Bundle size](/ts/reference/bundle-size) lists what each import costs.

## Let your bundler emit the worker

The player runs part of its work in a worker. It starts the worker from `player/worker-default.mjs`, a file inside the package, at `new URL('./player/worker-default.mjs', import.meta.url)`. Your bundler has to emit that file and keep the URL pointing at it. If the worker does not start, the player runs everything on the main thread.

### Vite

Add `moqtail` to `optimizeDeps.exclude` in `vite.config.ts`:

```ts
import { defineConfig } from 'vite'

export default defineConfig({
  optimizeDeps: { exclude: ['moqtail'] },
})
```

The dev server of Vite 7 and earlier pre-bundles the package into `node_modules/.vite/deps`, where the worker's URL points at a file that is not there. Vite 8 keeps the URL, and the setting does no harm. `vite build` emits the worker either way.

### Other bundlers

A bundler that handles `new Worker(new URL(…, import.meta.url), { type: 'module' })` in dependencies emits the worker by itself. With one that does not, build the worker from an entry of your own:

```ts
import { serveWorker } from 'moqtail/player/worker'

serveWorker()
```

Have the bundler build it to a file of its own, then pass a function that starts it in the player's `worker` option. On `<moqtail-video>`, player options go in `config`:

```ts
import { MOQtailPlayer } from 'moqtail/player'
import type { MOQtailVideoElement } from 'moqtail/player/component'

const worker = () => new Worker('/assets/player.worker.js', { type: 'module' })

const player = new MOQtailPlayer({ worker })
document.querySelector<MOQtailVideoElement>('moqtail-video')!.config = { worker }
```

## Allow the player in a Content Security Policy

Under a Content Security Policy, the player needs these sources. A directive the policy leaves out falls back to `default-src`.

| Directive     | Source                                                                                                                                 | Needed for                                                           |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `connect-src` | The relay's `https://` origin, such as `https://relay.moqtail.dev`. Add the port when it is not 443: `https://relay.example.com:4433`. | The WebTransport session                                             |
| `media-src`   | `blob:`                                                                                                                                | `<video>` in Chrome, which plays the media source from a `blob:` URL |
| `script-src`  | `blob:`                                                                                                                                | Audio on a `<canvas>`, whose audio worklet loads from a `blob:` URL  |
| `worker-src`  | Where the worker file is served from, usually `'self'`                                                                                 | The worker. Without `worker-src`, `script-src` applies.              |

This policy plays both `<video>` and `<canvas>` from the public relay:

```
default-src 'self'; connect-src 'self' https://relay.moqtail.dev; media-src 'self' blob:; script-src 'self' blob:
```

A blocked worker is not an error: the player plays on the main thread. A blocked audio worklet stops `<canvas>` playback of a stream with audio, with the error `MEDIA_FAILURE`.

## Check browser support

The player needs:

- **WebTransport**, in Chrome, and in Safari 26.4 or later. Browsers allow it only on `https://` pages and `localhost`.
- **Media Source Extensions** for a `<video>` and `<moqtail-video>`. The player takes `ManagedMediaSource` where the browser has it, as Safari does, and `MediaSource` elsewhere.
- **WebCodecs** and **AudioWorklet** for a `<canvas>`.

It does not need cross-origin isolation, so the page needs no `Cross-Origin-Opener-Policy` or `Cross-Origin-Embedder-Policy` headers.

## More information

- [Quickstart](/guide/quickstart), a page that plays a stream with `<moqtail-video>`
- [Run the player in a worker](/ts/player/worker)
- [`MOQtailPlayerOptions`](/ts/api/player/interfaces/MOQtailPlayerOptions), with `worker` and `preferWorker`
- [`serveWorker`](/ts/api/player/worker/functions/serveWorker)
- [Bundle size](/ts/reference/bundle-size)
