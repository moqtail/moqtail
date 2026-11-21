---
description: Play a live stream from the public MOQtail relay in a web page, in about five minutes.
---

# Quickstart

In this tutorial you build a web page that plays a live stream from the public MOQtail relay. You use `<moqtail-video>`, an element that works like a `<video>` but takes a `moqt://` URL.

## What you need

- Node.js 22 or later.
- Chrome, or Safari 26.4 or later. The player connects over WebTransport.

## 1. Create a project

```sh
npm create vite@latest my-player -- --template vanilla-ts
cd my-player
```

Vite creates a small TypeScript app in the `my-player` folder.

## 2. Install the package

```sh
npm install moqtail custom-media-element media-tracks
```

`custom-media-element` and `media-tracks` are optional peer dependencies of `moqtail`. Only `<moqtail-video>` needs them.

## 3. Keep Vite from pre-bundling moqtail

Create `vite.config.ts` next to `package.json`:

```ts
import { defineConfig } from 'vite'

export default defineConfig({
  optimizeDeps: { exclude: ['moqtail'] },
})
```

The player runs part of its work in a worker that it loads from a file inside the package. When Vite pre-bundles the package, that file's URL is lost.

## 4. Add the player

In `index.html`, replace `<div id="app"></div>` with a `<moqtail-video>`. Then replace everything in `src/main.ts`:

::: code-group

```html [index.html]
<body>
  <moqtail-video
    src="moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog"
    autoplay
    muted
    playsinline
    controls
    style="width: 100%"
  ></moqtail-video>
  <script type="module" src="/src/main.ts"></script>
</body>
```

```ts [src/main.ts]
import { defineMOQtailVideo } from 'moqtail/player/component'

defineMOQtailVideo()
```

:::

`defineMOQtailVideo()` registers the element. Its `src` is an MSF URL: the relay's address, then `#msf:` and the track to start from, here the catalog of the stream `moqtail-ch07`.

## 5. Run it

```sh
npm run dev
```

Open the address Vite prints, usually `http://localhost:5173`. After a second or two the live stream plays. It starts muted, because browsers only autoplay muted video. Use the controls to unmute it.

## What you built

Your page opens a WebTransport session to `relay.moqtail.dev`, reads the stream's catalog and subscribes to a video and an audio track. While it plays, the element keeps playback close to the live edge and switches between renditions as your bandwidth changes.

## Next steps

- [Build a live player](/ts/tutorial) with the `MOQtailPlayer` API, events and a `<canvas>`.
- [Run your own relay](/relay/tutorial) and point the player at it.
- Read [how the player works](/guide/concepts/player).
