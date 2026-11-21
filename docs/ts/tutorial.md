---
description: Build a web page that plays live and VOD streams with MOQtailPlayer, shows its state, pauses, seeks, draws on a canvas and plays from your own relay.
---

# Build a live player

In this tutorial you replace the `<moqtail-video>` element from the [quickstart](/guide/quickstart) with `MOQtailPlayer`, the API behind it. You show the player's state on the page, add a pause button, seek in a VOD stream, play a stream on a `<canvas>`, and finally play a stream from your own relay.

## What you need

- The `my-player` project from the [quickstart](/guide/quickstart), with `moqtail` installed and its `vite.config.ts`.
- Chrome, or Safari 26.4 or later.

## 1. Play on a `<video>`

In `index.html`, replace `<div id="app"></div>` with a `<video>`. Then replace everything in `src/main.ts`:

::: code-group

```html [index.html]
<body>
  <video muted playsinline style="width: 100%"></video>
  <script type="module" src="/src/main.ts"></script>
</body>
```

```ts [src/main.ts]
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!

const player = new MOQtailPlayer({ plugins: [liveBuffer(), abr()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

player.start()
```

:::

`source()` takes the MSF URL to play and `sink()` the element to play it on. `start()` connects to the relay, reads the catalog, and starts playback.

`MOQtailPlayer` installs no plugins by itself. `liveBuffer()` keeps playback close to the live edge, and `abr()` switches renditions as your bandwidth changes. They are the two plugins `<moqtail-video>` installs for you.

Run `npm run dev` and open the address Vite prints. The live stream plays as in the quickstart, without controls. It is muted, so the browser lets it start without a click.

## 2. Show the player's state

Replace both files with this version. The highlighted lines are new:

::: code-group

```html [index.html] {3-4}
<body>
  <video muted playsinline style="width: 100%"></video>
  <p>State: <span id="state">idle</span></p>
  <p id="error"></p>
  <script type="module" src="/src/main.ts"></script>
</body>
```

```ts [src/main.ts] {4-5,11-16,18-19}
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const stateText = document.querySelector('#state')!
const errorText = document.querySelector('#error')!

const player = new MOQtailPlayer({ plugins: [liveBuffer(), abr()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

player.on('statechange', (state) => {
  stateText.textContent = state
})
player.on('error', (error) => {
  errorText.textContent = `${error.code}: ${error.message}`
})

// A failure also arrives as the `error` event
player.start().catch(() => {})
```

:::

The listeners go before `start()`, so they see every state from the first one. The page reloads and shows `connecting`, then `ready`, then `playing`.

To see an error, change `moqtail-ch07` in the URL to `moqtail-nope`, a namespace the relay doesn't have. After ten seconds the page shows `CATALOG_TIMEOUT: No catalog within 10000 ms` and the state `error`. Change the URL back before you go on.

## 3. Add a pause button

Add a button next to the state, and change its label with the state:

::: code-group

```html [index.html] {3}
<body>
  <video muted playsinline style="width: 100%"></video>
  <p>State: <span id="state">idle</span> <button id="toggle">Play</button></p>
  <p id="error"></p>
  <script type="module" src="/src/main.ts"></script>
</body>
```

```ts [src/main.ts] {5,14,20-23}
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const video = document.querySelector('video')!
const stateText = document.querySelector('#state')!
const toggle = document.querySelector('#toggle')!
const errorText = document.querySelector('#error')!

const player = new MOQtailPlayer({ plugins: [liveBuffer(), abr()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog')
  .sink(video)

player.on('statechange', (state) => {
  stateText.textContent = state
  toggle.textContent = state === 'playing' ? 'Pause' : 'Play'
})
player.on('error', (error) => {
  errorText.textContent = `${error.code}: ${error.message}`
})

toggle.addEventListener('click', () => {
  if (player.state === 'playing') player.pause()
  else player.play()
})

// A failure also arrives as the `error` event
player.start().catch(() => {})
```

:::

Click **Pause**: the video stops and the state is `paused`. Wait a few seconds and click **Play**. The stream doesn't go on from where it stopped: `liveBuffer()` jumps back to about a second behind the live edge.

## 4. Seek in a VOD stream

The public relay also has a recording of the same channel. In `source()`, change the URL to its catalog:

```ts
  .source('moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog')
```

Then add a button that skips 30 seconds ahead. In `index.html`, replace the line with the state and the pause button with:

```html
<p>State: <span id="state">idle</span> <button id="toggle">Play</button> <button id="skip">+30 s</button></p>
```

and give it a listener in `src/main.ts`, after the pause button's one:

```ts
document.querySelector('#skip')!.addEventListener('click', () => {
  player.seek(video.currentTime + 30)
})
```

The recording starts from its beginning, and the video's own clock counts from 0. Click **+30 s** and the video jumps ahead 30 seconds. `seek()` takes seconds of media time, the same clock as `video.currentTime`.

The stream is on demand, so the player doesn't attach `liveBuffer()` and `abr()`: both act on live streams only. Pausing and playing again goes on from where you paused.

## 5. Play on a `<canvas>`

A `<video>` plays CMAF. A `<canvas>` plays LOC: the player decodes the frames with WebCodecs, draws the video and plays the audio through Web Audio. The public test stream `moqtail-testsrc` comes in both packagings.

Replace both files with this version of step 3. It has a `<canvas>` in place of the `<video>`, and the test stream's URL:

::: code-group

```html [index.html] {2}
<body>
  <canvas style="width: 100%"></canvas>
  <p>State: <span id="state">idle</span> <button id="toggle">Play</button></p>
  <p id="error"></p>
  <script type="module" src="/src/main.ts"></script>
</body>
```

```ts [src/main.ts] {3,9-10}
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

const canvas = document.querySelector('canvas')!
const stateText = document.querySelector('#state')!
const toggle = document.querySelector('#toggle')!
const errorText = document.querySelector('#error')!

const player = new MOQtailPlayer({ plugins: [liveBuffer(), abr()] })
  .source('moqt://relay.moqtail.dev#msf:moqtail-testsrc--catalog')
  .sink(canvas)

player.on('statechange', (state) => {
  stateText.textContent = state
  toggle.textContent = state === 'playing' ? 'Pause' : 'Play'
})
player.on('error', (error) => {
  errorText.textContent = `${error.code}: ${error.message}`
})

toggle.addEventListener('click', () => {
  if (player.state === 'playing') player.pause()
  else player.play()
})

// A failure also arrives as the `error` event
player.start().catch(() => {})
```

:::

The test stream's color bars play on the canvas. They may start silent, because browsers let a page start audio only after a click on it. Click **Pause** and then **Play** to hear its 220 Hz tone.

A canvas plays live streams only, and it can't seek back. To try the VOD stream again, go back to the `<video>`.

## 6. Play from your own relay

The player needs a relay that carries an MSF catalog and the media tracks it lists. The CLI client in the relay tutorial publishes test objects, not media, so here you publish from the **Publish** tab of player.moqtail.dev, which can send to any relay.

1. Start a relay as in [Run your own relay](/relay/tutorial), so that it listens on `moqt://localhost:4433` with a certificate your browser trusts.
2. Open [player.moqtail.dev](https://player.moqtail.dev) and select **Publish**. Set **Relay URL** to `moqt://localhost:4433` and **Namespace** to `moqtail/tutorial`. Select **Test Source**, then **Go Live**. If Chrome asks whether the page may connect to devices on your local network, allow it.
3. The Publish tab sends CMAF, so play it on a `<video>`. Go back to the step 4 versions of `index.html` and `src/main.ts`, and change the URL in `source()`:

   ```ts
     .source('moqt://localhost:4433#msf:moqtail-tutorial--catalog')
   ```

Your page plays the test source through your relay. The URL names the namespace's fields joined by `-`, then `--` and the catalog track. A field that itself has a `-` in it, such as the `blue-fox` the Publish tab suggests, is written differently: [MSF URLs](/ts/reference/msf-url) has the rules, and the tab's **Watch URL** has your URL in its `url` parameter.

## What you built

Your page builds the player chain itself: a source for the catalog and a sink for the element you chose. It follows the player through its states, controls playback, and seeks in on-demand streams. It plays CMAF on a `<video>` and LOC on a `<canvas>`, from the public relay or from yours.

## Next steps

- [Use `MOQtailPlayer`](/ts/player/player): every option, sink option and control call.
- [Handle states and errors](/ts/player/errors): what each state and error code means.
- [Tune live latency](/ts/player/latency) with `liveBuffer()`.
- Read [how the player works](/guide/concepts/player).
