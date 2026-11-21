---
description: What the moqtail package holds, which of its three layers to use, its entry points, and what it plays in which browser.
---

# TypeScript library

The `moqtail` package is MOQtail's browser side. It has three layers, each built on the one below it:

- **`<moqtail-video>`** is a custom element that works like a `<video>` but takes an MSF URL. It builds and runs a player for you.
- **`MOQtailPlayer`** turns an MSF URL into a chain of elements that request tracks, transform media and play it. You choose its plugins and where the media goes: a `<video>` or a `<canvas>`.
- **`MOQtailClient`** is one MOQT session with a relay. It subscribes, fetches and publishes, and knows nothing about media.

## Which layer to use

**`<moqtail-video>`** fits wherever a `<video>` does. It forwards the media element API, so a player UI that drives a `<video>` can drive it too: [player.moqtail.dev](https://player.moqtail.dev) wraps it in a Video.js skin. It installs the live buffer and ABR plugins for you and lists the renditions the way media UIs expect. Most pages that only play a stream need nothing else.

**`MOQtailPlayer`** is the layer under the element, and `video.player` is the one the element built. Use it directly for what the element doesn't do: play LOC on a `<canvas>`, add sources or transforms of your own with `registerElement()` and `transform()`, or decide yourself when to connect and when to play. You get the same states and events either way.

**`MOQtailClient`** is for code that is not a media player: publishing a camera, reading data tracks, discovering namespaces, or testing a relay. The player uses the same client underneath, and every player on a page shares one session.

## Entry points

Import from the subpath that holds what you need. The package is ES modules only.

| Entry                       | What it holds                                                                                                         |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `moqtail`                   | Everything in `moqtail/client`, `moqtail/model` and `moqtail/util`, but not the player                                |
| `moqtail/client`            | `MOQtailClient`, its options, events and request results                                                              |
| `moqtail/model`             | The MOQT wire model: control messages, objects, parameters, properties, and names such as `FullTrackName` and `Tuple` |
| `moqtail/util`              | Log levels (`setLogLevel()`, `LogLevel`) and `Emitter`                                                                |
| `moqtail/player`            | `MOQtailPlayer`, its built-in elements and plugins (`liveBuffer()`, `abr()`, `ssts()`), and the API to write your own |
| `moqtail/player/component`  | `<moqtail-video>`: `defineMOQtailVideo()` and `MOQtailVideoElement`                                                   |
| `moqtail/player/elements/*` | `MSE`, `Playout`, `WebCodecs` and `Locmaf`, one per entry, bundled with your app instead of loaded on first use       |
| `moqtail/player/worker`     | `serveWorker()`, for a worker script of your own that runs your elements' engines                                     |

Only `moqtail/player/component` needs the optional peer dependencies `custom-media-element` and `media-tracks`.

## Browser support

The package connects over WebTransport, so it runs in Chrome, and in Safari 26.4 or later. The player also needs Media Source Extensions to play on a `<video>`, and WebCodecs and AudioWorklet to play on a `<canvas>`. [Install the library](/ts/install) has the details, including bundler settings and content security policy.

## What plays where

The sink you give the player decides which packaging it plays:

| Sink       | Plays        | Streams                         |
| ---------- | ------------ | ------------------------------- |
| `<video>`  | CMAF, LOCMAF | Live and VOD, with seeking      |
| `<canvas>` | LOC          | Live only, seeking forward only |

When a catalog offers the same content in more than one packaging, the player keeps the one its sink can play. `<moqtail-video>` always plays on a `<video>`.

Captions are not played yet. The player picks one audio language when it starts, from `preferredLanguage` or the browser's language, and can't switch language during playback yet.

## Related

- [How the player works](/guide/concepts/player): the chain of elements, plugins, the worker and the shared session.
- [Catalogs and packaging](/guide/concepts/media): MSF catalogs, CMAF and LOC.
- [Quickstart](/guide/quickstart): play a stream with `<moqtail-video>` in five minutes.
- [API reference](/ts/api/)
