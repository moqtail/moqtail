# moqtail-ts

## 0.18.0

### Minor Changes

- [#498](https://github.com/moqtail/moqtail/pull/498) [`587a808`](https://github.com/moqtail/moqtail/commit/587a8088c48a3c4d2af419c32f02ce639beebf75) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Add the CODEC_STRING (0x11) LOC property from draft-ietf-moq-loc-05, and `Timestamp.toMicros()`, which applies the draft's Timescale rule (µs since the Unix epoch when no Timescale is present).

- [`65261cb`](https://github.com/moqtail/moqtail/commit/65261cbec2ae56677fc1a05d74fbcba1c10100ea) Thanks [@DenizUgur](https://github.com/DenizUgur)! - `liveBuffer()` takes `tolerance`, `stallThreshold`, `minRate`, `jumpFactor` and `jumpTolerance` alongside `catchupRate` and `targetLatency`, so every value the controller tunes with is configurable. The defaults are unchanged.

  The playout sink takes a `preroll` option, `player.sink(canvas, { preroll })`: the media seconds it queues before playback starts or restarts after a stall (default 0.5). Lower it together with `stallThreshold` to hold a forward buffer under 0.5 s.

- [#499](https://github.com/moqtail/moqtail/pull/499) [`0910636`](https://github.com/moqtail/moqtail/commit/091063621ebbc4d897d16eec7765b72556e0b772) Thanks [@DenizUgur](https://github.com/DenizUgur)! - A built-in `loc` source reads LOC switching sets (draft-ietf-moq-loc-05). Its pads carry `loc` packets, with the WebCodecs decoder config (`decoderConfig`), the init data and the track's Timescale (`timescale`) as pad properties.

- [#502](https://github.com/moqtail/moqtail/pull/502) [`fd2f2e5`](https://github.com/moqtail/moqtail/commit/fd2f2e563b4761bd7727a23a1e05342b300fbfd0) Thanks [@DenizUgur](https://github.com/DenizUgur)! - The chain loads the transforms a pad needs to reach a sink, after the declared elements and in the fewest steps, so LOC catalogs play with `player.sink(canvas)`. The catalog keeps only the switching sets a declared sink can reach, and an element definition's optional `supports(track)` drops renditions it can't handle: `mse` and `webcodecs` ask the browser. Element contexts gain `cost(track)`.

  Breaking: `player.transform()` takes a transform element definition and its options, like `sink()`, instead of an instance. The package is pre-1.0, so there is no compatibility overload.

- [#497](https://github.com/moqtail/moqtail/pull/497) [`d5ca91f`](https://github.com/moqtail/moqtail/commit/d5ca91f4c722e59e505ca8a5f1fe649191e4f69b) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Pads carry a `Packet` (the track's rendition id, location, object properties and payload) instead of a `MoqtObject`: custom elements read `item.packet` in `receive()` and write `{ type: 'packet', packet }`.

- [#501](https://github.com/moqtail/moqtail/pull/501) [`5b2a82a`](https://github.com/moqtail/moqtail/commit/5b2a82a518bd551ca3392397f7f1826da827b4de) Thanks [@DenizUgur](https://github.com/DenizUgur)! - A `playout` sink, through `player.sink(canvas)`, plays `raw` audio through an AudioWorklet and draws `raw` video on a canvas in sync with it. It keeps three decoded frames, and its `buffered` counts what the decoder holds encoded, so `liveBuffer()` holds the configured buffer through its playback rate, on WebKit too, and its forward seeks. It dispatches `waiting` when video or audio arrives later than its frame rate or sample rate allows, and `playing` once it's back. It refuses non-live streams, and a seek back throws.

- [`c87a3a3`](https://github.com/moqtail/moqtail/commit/c87a3a3a965aeab22241e838510134d539efd433) Thanks [@DenizUgur](https://github.com/DenizUgur)! - `MediaSinkElement` has a `preciseRate` flag: whether `setPlaybackRate()` keeps the clock running. `liveBuffer()` holds latency with jumps only on a sink without one, which the `mse` sink is on WebKit. A custom media sink sets it.

- [#500](https://github.com/moqtail/moqtail/pull/500) [`963a45d`](https://github.com/moqtail/moqtail/commit/963a45da15aa27f92c33285b7da16070b2295581) Thanks [@DenizUgur](https://github.com/DenizUgur)! - A built-in `webcodecs` transform decodes a `loc` pad into a `raw` pad of `VideoFrame`s or `AudioData`, one pad per instance. Pads carry these as `{ type: 'frame', frame }` items: a `Frame` has the `track`, `location` and `properties` of the object it was decoded from, and the receiver closes its `payload`. It decodes only once the consumer took the frame before: a frame's write resolves once it's taken, so packets wait encoded in the transform rather than as frames pinning decoder buffers. It sends the seconds it holds downstream as a `{ type: 'held', seconds }` pad signal whenever they change, and holds the source back past 10 s.

- [#504](https://github.com/moqtail/moqtail/pull/504) [`f8ba3c5`](https://github.com/moqtail/moqtail/commit/f8ba3c511715cc6522151a75c947d6a1f070ca34) Thanks [@DenizUgur](https://github.com/DenizUgur)! - The player runs `webcodecs` decoding and `playout` canvas rendering in a worker of its own where the browser allows it, linking them inside the worker so decoded frames stay off the main thread. `preferWorker: false` keeps them on the main thread. `serveWorker` from `moqtail/player/worker`, with the `worker` player option, runs your own elements' engines in that worker: an element's `threads` probe gives it `context.worker`. `ElementContext.fail` reports an element's fatal error from outside `receive()`.

### Patch Changes

- [`51e404e`](https://github.com/moqtail/moqtail/commit/51e404e3f3c519c90b8bbd10b1c8a7d13e9b190c) Thanks [@DenizUgur](https://github.com/DenizUgur)! - `publishDone()` reports `TRACK_ENDED` when no status code is given, matching its default reason phrase. It reported `INTERNAL_ERROR`.

- [`51e404e`](https://github.com/moqtail/moqtail/commit/51e404e3f3c519c90b8bbd10b1c8a7d13e9b190c) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Subscriber priority is rounded and clamped to 0-255. A fractional value no longer disconnects the session.

- [`51e404e`](https://github.com/moqtail/moqtail/commit/51e404e3f3c519c90b8bbd10b1c8a7d13e9b190c) Thanks [@DenizUgur](https://github.com/DenizUgur)! - A subscription's stream ends on `unsubscribe()` and when the session ends, so a loop reading it finishes instead of waiting forever.

- [`51e404e`](https://github.com/moqtail/moqtail/commit/51e404e3f3c519c90b8bbd10b1c8a7d13e9b190c) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Setting `config.plugins = []` on `<moqtail-video>` installs no plugins. It installed `liveBuffer()` and `abr()`, which are now installed only when `config.plugins` is unset.

## 0.17.0

### Minor Changes

- [#476](https://github.com/moqtail/moqtail/pull/476) [`09d667c`](https://github.com/moqtail/moqtail/commit/09d667c4932000dff4ba18eb0013b07469adecb9) Thanks [@zafergurel](https://github.com/zafergurel)! - React to GOAWAY instead of waiting for the relay to close the session. The Rust test client logs the GOAWAY's new session URI and timeout, stops its command and closes the session; subscribe and publish commands then reconnect to the new session URI and run again, and leave when none is given. The player leaves the session on GOAWAY and reopens it at the new session URI (or the current one when none is given), so players see `reconnecting` and `reconnected` as for a lost session.

- [#479](https://github.com/moqtail/moqtail/pull/479) [`d949ba2`](https://github.com/moqtail/moqtail/commit/d949ba236afd8eacfee15946918c16e2d316a855) Thanks [@DenizUgur](https://github.com/DenizUgur)! - The player plays LOCMAF tracks (draft-einarsson-moq-locmaf-01, `locmafVersion` 0.3): a built-in `locmaf` source rebuilds each object into its canonical CMAF chunk for the MSE sink. Where a catalog offers the same media as CMAF and LOCMAF, the LOCMAF switching set is chosen; the two never share a set.

### Patch Changes

- [#480](https://github.com/moqtail/moqtail/pull/480) [`e278673`](https://github.com/moqtail/moqtail/commit/e2786739384d953ab2208086fccd6ad900d95787) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Implement the ABR controller: `abr()`, which `<moqtail-video>` installs by default, is now `MlAbrController` (replacing `DefaultAbrController`), a small neural network that switches live renditions and probes for room with FETCHes below live priority. Live switching sets now start at their middle bitrate. Also improve stuck playback scenarios by nudging the playback so that it lands inside a buffered region.

- [`d2b115c`](https://github.com/moqtail/moqtail/commit/d2b115cb5a3897900630e66892b043ef73bdbdcf) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Mark the package `sideEffects: false` so bundlers can tree-shake unused modules.

- [#482](https://github.com/moqtail/moqtail/pull/482) [`e2b467a`](https://github.com/moqtail/moqtail/commit/e2b467a50e3efc58efc5f7c3f1cf08219aa013a0) Thanks [@zafergurel](https://github.com/zafergurel)! - Encode the TRACK_NAMESPACE_PREFIX parameter in REQUEST_UPDATE as a bare Track Namespace, with the field count right after the parameter type. It was written with a length in front, so prefix updates from other implementations failed to parse at the relay.

- [#491](https://github.com/moqtail/moqtail/pull/491) [`254f577`](https://github.com/moqtail/moqtail/commit/254f577fb2aa36847bc1e6ca6d0b265ad4fd835a) Thanks [@zafergurel](https://github.com/zafergurel)! - Reject received Objects that carry a Mandatory Track Property (other than a GREASE value), directly or inside Immutable Properties, or whose property block is malformed. A truncated property block is now a protocol violation instead of stalling the data stream while it waits for bytes that will never come.

- [#492](https://github.com/moqtail/moqtail/pull/492) [`2623df4`](https://github.com/moqtail/moqtail/commit/2623df4271b5f7459c910cc136f50d1d35b6395d) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Make `<moqtail-video>` follow the catalog's `accessibleWindowTime` (MSF 5.2.46) on live streams. With a window, its `currentTime`, `seekable`, `buffered` and `liveEdgeStart` count from the window's start, so seekbars that start at 0, such as Video.js's, cover only the accessible range, and `targetLiveWindow` is `Infinity`. Without a window there is no DVR: `seekable` is empty, `targetLiveWindow` is `0`, and setting `currentTime` is ignored. The MSE sink bounds the live seekable range by the window even without a media timeline. client-js hides the Video.js time slider while nothing is seekable, leaving a spacer in its place as the live skin does, and its skin styles move to `videojs.css`.

- [`3cd43a3`](https://github.com/moqtail/moqtail/commit/3cd43a375eeada367be7bd8d5c08d35e5527e82e) Thanks [@DenizUgur](https://github.com/DenizUgur)! - add support for msf draft-02

- [#488](https://github.com/moqtail/moqtail/pull/488) [`68d08b9`](https://github.com/moqtail/moqtail/commit/68d08b989de649e34baf8da7371683fbf5301ba3) Thanks [@zafergurel](https://github.com/zafergurel)! - Deliver the objects of tracks the peer pushes with PUBLISH again; their data streams failed with "Cannot read properties of undefined (reading 'objects')". A pushed track's stream now also ends on its PUBLISH_DONE. `allocatePseudoRequestId()` is removed.

- [#493](https://github.com/moqtail/moqtail/pull/493) [`4d5a57e`](https://github.com/moqtail/moqtail/commit/4d5a57e8c548a488bfc219a8828893d6b05544d2) Thanks [@DenizUgur](https://github.com/DenizUgur)! - The live buffer controller no longer speeds up or jumps back to live after a seek behind the live edge, and resumes following the edge after a seek to it (the Live button, or the end of the seekbar).

## 0.16.0

### Minor Changes

- [#463](https://github.com/moqtail/moqtail/pull/463) [`b14f129`](https://github.com/moqtail/moqtail/commit/b14f1298ac6d55578421af83b02e09eb4dfced49) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Breaking: `MOQtailClient` events take any number of listeners through `client.on(event, listener)` and `client.off()`. `new MOQtailClient(options)` does no I/O and `await client.init()` connects; `MOQtailClient.new()`, the `callbacks` option and the `on*` properties are removed. `client.subscribe` returns `{ requestId, subscribeOk, stream, done }` and `client.fetch` returns `{ requestId, fetchOk, stream }`.

  - An `ordering` option on the client, `subscribe` and `fetch` releases objects in location order across streams and datagrams, reporting what it stopped waiting for with `orderinggap`.
  - `client.switch` returns the new `SubscribeResult` and no longer sends FORWARD; `subscribeUpdate` only sends priority and forward when given.
  - `client.trackNameOf(headerEvent)` names the track a data stream carries.
  - `client.clientSetup` holds the SETUP the client sent, next to `client.serverSetup`.
  - `LiveTrackSource` no longer loses the last object before PUBLISH_DONE, and a closing session leaves no unhandled rejections.
  - `moqt://` is matched case-insensitively.

- [#463](https://github.com/moqtail/moqtail/pull/463) [`812e732`](https://github.com/moqtail/moqtail/commit/812e7324b4e3c110ad2f545956fe85efeeeaa35a) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Add `moqtail/player`, an MSF/CMSF player built from elements, and the `<moqtail-video>` custom element in `moqtail/player/component`.

  Breaking: the old catalog model (`CMSFCatalog`, `CMSF`, …) is removed. Read catalogs through the player's `catalog` event and type published ones as `CatalogJson`.

  - `MOQtailPlayer` builds a source → transform → sink chain, shares one reconnecting client across players on a page, and reports `statechange`, `error` and `warning`.
  - An element runtime (`defineElement`, source/transform/sink base classes, pads with backpressure) for custom elements.
  - Built-in catalog source with delta updates, a CMAF source with seeking, URL ranges and rendition switching, and an MSE sink.
  - Media and event timelines and per-request object ordering.
  - Plugins (`definePlugin`, `opts.plugins`) that add setup parameters, check the peer's SETUP, and offer elements, switching set sources, buffer controllers and ABR controllers. Built in: `ssts()`, whose sources provide `ABR`, `liveBuffer()` and `abr()`. A `selectionchange` event follows `selectRenditions()`, and `player.isAbrAvailable()` says whether a source switches automatically.
  - `parseMsfUrl` and `formatMsfUrl` for `moqt://...#msf:` URLs.

### Patch Changes

- [#465](https://github.com/moqtail/moqtail/pull/465) [`a116ca7`](https://github.com/moqtail/moqtail/commit/a116ca785c9daaa4ce3ba68612df82f69a6c365d) Thanks [@zafergurel](https://github.com/zafergurel)! - Datagrams work on Safari: when the transport has no `datagrams.writable` (Safari implements only `datagrams.createWritable()`), the client sends through `createWritable()` instead of failing to connect.

- [#464](https://github.com/moqtail/moqtail/pull/464) [`9e59347`](https://github.com/moqtail/moqtail/commit/9e59347a544ab3b7cac703ec5d72aa55f6ab43d6) Thanks [@zafergurel](https://github.com/zafergurel)! - A publication no longer ends when the peer stops one of its subgroup streams (STOP_SENDING). The rest of that group is skipped and later groups are still sent.

## 0.15.0

### Minor Changes

- [#444](https://github.com/moqtail/moqtail/pull/444) [`8c71f44`](https://github.com/moqtail/moqtail/commit/8c71f447385902e7e9d35f15cddb0f276fbc7f75) Thanks [@zafergurel](https://github.com/zafergurel)! - Serve FETCH from the relay's cache whenever the cache accounts for every Object of a group, and fetch upstream only what it does not. The relay now creates a track for a FETCH of a track it does not have yet, keeps a track for its cached Objects after its upstream goes away, records which Objects are known not to exist (from End of Group and End of Track statuses, gap properties, a group's last datagram or subgroup stream, and what an upstream FETCH response passes over), and lets concurrent FETCHes of the same range share one upstream FETCH. A PUBLISH_DONE now waits for a track's datagrams to go quiet. Both libraries parse End of Timed-Out Range markers and hand End of Range markers to the application (`RecvDataStream::next_item` in moqtail-rs, an `onEndOfRange` handler in moqtail). The test client can publish groups that already exist at startup, reset streams, drop datagrams, send End of Group and End of Track objects, and cut or mark its FETCH responses.

- [#459](https://github.com/moqtail/moqtail/pull/459) [`561588a`](https://github.com/moqtail/moqtail/commit/561588ae232fe7660bc69ee2812a98231d6481d0) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix the LOC property ids: TIMESTAMP is 0x10 and VIDEO_FRAME_MARKING is 0x09, and AUDIO_CONFIG (0x0F) is added. Video frame marking now carries the RFC 9626 flag byte instead of a varint, so `addVideoFrameMarking` takes `isKey: boolean` and receivers check `independent` (`is_independent_frame()` in moqtail-rs). Timestamps are microseconds; meet and client-js publish them that way.

## 0.14.0

### Minor Changes

- [#386](https://github.com/moqtail/moqtail/pull/386) [`d642d33`](https://github.com/moqtail/moqtail/commit/d642d335d1f0b0bc4d2020dc47416eaeb2b4a550) Thanks [@zafergurel](https://github.com/zafergurel)! - Switch tracks with the SWITCH_FROM parameter

  A SUBSCRIBE or REQUEST_UPDATE carrying SWITCH_FROM activates its own
  subscription and suspends the one it names, in place of the SWITCH message,
  which is gone. A hard switch stops the suspended subscription at the cutover; a
  soft switch lets it drain to the group before the new subscription's start, so
  the two are contiguous.

  The fill filter types — AbsoluteStartFill, AbsoluteRangeFill and the new
  RelativeStartFill — ask the publisher for the objects already published, which
  arrive on a fill fetch stream: a unidirectional stream beginning with a
  FETCH_HEADER that names the request which asked for the fill. FILL_PARAMETERS
  overrides the subscriber priority and group order that stream runs at.

  FETCH is a single message now. Fetch Type and Joining Fetch are gone, along with
  INVALID_JOINING_REQUEST_ID, whose codepoint INVALID_SWITCH takes over.

- [#440](https://github.com/moqtail/moqtail/pull/440) [`55eaa12`](https://github.com/moqtail/moqtail/commit/55eaa1234fd394c13ae0c2ea0f50e60e66605578) Thanks [@zafergurel](https://github.com/zafergurel)! - Add `onDataEvent`, which reports every data stream as lightweight records: its header when it opens, each object's IDs and payload length, and how it ended (fin, reset with its code, or error). Each record carries the local time it happened, and object records carry the LOC Timestamp property when present, for interarrival, jitter and latency measurements. Payloads are never handed over. It can be passed in `MOQtailClient.new({ callbacks })`.

### Patch Changes

- [#409](https://github.com/moqtail/moqtail/pull/409) [`1fbc397`](https://github.com/moqtail/moqtail/commit/1fbc39744e1802bbcaa9d9e6208218c43bec0c23) Thanks [@burak-kara](https://github.com/burak-kara)! - Close the WebTransport session in `disconnect()`: the guard tested the `closed` Promise, so `close()` was never called and the session stayed open at the relay

- [#440](https://github.com/moqtail/moqtail/pull/440) [`55eaa12`](https://github.com/moqtail/moqtail/commit/55eaa1234fd394c13ae0c2ea0f50e60e66605578) Thanks [@zafergurel](https://github.com/zafergurel)! - Report request-stream traffic through `onMessageSent` and `onMessageReceived`: only the control stream fired them, so SUBSCRIBE, PUBLISH, FETCH, the namespace messages and their replies went unreported

- [#407](https://github.com/moqtail/moqtail/pull/407) [`9d278d4`](https://github.com/moqtail/moqtail/commit/9d278d49157ec6f48292b0b36fff41dd46cbf8c0) Thanks [@burak-kara](https://github.com/burak-kara)! - Fix the Object ID Delta of the object after Object ID 0 in a subgroup: Object ID 1 was sent with delta 1, so receivers read it as 2

- [#440](https://github.com/moqtail/moqtail/pull/440) [`55eaa12`](https://github.com/moqtail/moqtail/commit/55eaa1234fd394c13ae0c2ea0f50e60e66605578) Thanks [@zafergurel](https://github.com/zafergurel)! - Serialize the FETCH header on outgoing fetch streams, which was written as an object instead of bytes; report outgoing data through `onDataSent`, which was never wired; and stop reporting each incoming data stream header twice through `onDataReceived`

## 0.13.0

### Minor Changes

- [#391](https://github.com/moqtail/moqtail/pull/391) [`fbec509`](https://github.com/moqtail/moqtail/commit/fbec5094b0be8fbb1431801093d091444c331a53) - - Changes related to draft-18 compatibility
  - Added SSTS (Sender Side Track Switching) support, gated behind `--ssts-enable` since the setup option and the message parameter it uses are provisional on an unadopted draft. Also implements the default ABR algorithm and a backpressure based experimental algorithm. Algorithms live in the `moqtail-ssts` crate.
  - Rename warp to cmsf and make catalog version string
  - Refactor enums to use tryFrom for cleaner API
  - Keep a publication alive when the peer moves its Forward State, and report the move through onForwardStateChange
  - Tell onPeerPublishDone which request the PUBLISH_DONE ended
  - Fix(moqtail-ts): stop TerminationCode.tryFrom throwing on five valid enum values
  - Client: add a subscribe-namespace command, receive objects for the tracks SUBSCRIBE_TRACKS is handed, cancel a prefix subscription by resetting its request stream, and report REQUEST_ERROR with its code and reason
  - Handle malformed FETCH track on end client
  - Helper function to validate subgroup priority in FETCH
  - Check prior id gaps for malformed track
  - Relay: reset downstream FETCH on malformed upstream track
  - Relay: handle objects received before FETCH_OK
  - Relay: report stream closed for empty FETCHes

## 0.12.1

### Patch Changes

- [#211](https://github.com/moqtail/moqtail/pull/211) [`a229d93`](https://github.com/moqtail/moqtail/commit/a229d93183adfc508badcc3d886290e0ed013d49) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix `Header.deserialize` rejecting subgroup headers with default publisher priority. The dispatch previously used an explicit switch covering only `0x10–0x15` and `0x18–0x1D`, missing the `0x30–0x35` and `0x38–0x3D` ranges (DEFAULT_PRIORITY bit set). Replaced with bitmask detection matching the Rust validator.

## 0.12.0

### Minor Changes

- [#206](https://github.com/moqtail/moqtail/pull/206) [`9c236c9`](https://github.com/moqtail/moqtail/commit/9c236c9117578aaf1033ee1a8e7e52ad04086979) Thanks [@kerembkmz](https://github.com/kerembkmz)! - Normalize empty subgroup extension headers to null so [] serializes the same as no headers.

### Patch Changes

- [#208](https://github.com/moqtail/moqtail/pull/208) [`211ce94`](https://github.com/moqtail/moqtail/commit/211ce94bd1890cb4846d1805dc476e2b6702321b) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix Key-Value-Pair Type fields being encoded/decoded as raw absolute values instead of deltas from the previous Type in the same list, per draft-ietf-moq-transport-16 (#1315). This broke interop with spec-compliant v16 peers: parameters such as FORWARD or SUBSCRIPTION_FILTER could be received as the wrong type (e.g. logged as "Unsupported parameter 48/81") by any peer that correctly delta-decodes. Affects Setup/Message Parameters and Object/Track Extension Headers (including the nested ImmutableExtensions list) in both the Rust (`moqtail-rs`) and TypeScript (`moqtail`) libraries.

## 0.11.0

### Minor Changes

- [#189](https://github.com/moqtail/moqtail/pull/189) [`996f334`](https://github.com/moqtail/moqtail/commit/996f3347d28586b74d993654abdce40ed92d9a44) Thanks [@zafergurel](https://github.com/zafergurel)! - Early discard policy can be defined per subscription.

- [#191](https://github.com/moqtail/moqtail/pull/191) [`a9c4431`](https://github.com/moqtail/moqtail/commit/a9c4431e6995af87333ba64bda934fd75b6b1b6a) Thanks [@zafergurel](https://github.com/zafergurel)! - Add `isValidTrackAlias` type guard in `src/client/util/validators.ts`. Thanks to [thexeos](https://github.com/thexeos) for pointing out the bug and proposing the fix.

## 0.10.1

### Patch Changes

- [#184](https://github.com/moqtail/moqtail/pull/184) [`ce9959e`](https://github.com/moqtail/moqtail/commit/ce9959e162b2a4eddad377d47a649397e3a5c8ae) Thanks [@zafergurel](https://github.com/zafergurel)! - add setEarlyDiscardPolicy to cancel slow subgroup streams after a configurable timeout

## 0.10.0

### Minor Changes

- [#145](https://github.com/moqtail/moqtail/pull/145) [`1b855cf`](https://github.com/moqtail/moqtail/commit/1b855cfece77cbade63f8263f485b8b5c7839134) Thanks [@zafergurel](https://github.com/zafergurel), [@fatih-alperen](https://github.com/fatih-alperen), [@ctllmp](https://github.com/ctllmp), [@beyzademirr](https://github.com/beyzademirr)! - Implement MOQ Transport draft-16 compliance across all packages.
  - New ALPN-based session setup flow (ClientSetup / ServerSetup)
  - Replaced VersionParameter with MessageParameter; added typed parameters: DeliveryTimeout, Expires, Forward, GroupOrder, LargestObject, NewGroupRequest, SubscriberPriority, SubscriptionFilter
  - Implemented TrackExtension and ObjectExtension; updated Publish, Subscribe, Fetch, PublishOk, SubscribeOk, FetchOk messages
  - Unified datagram wire format (Datagram replaces DatagramObject and DatagramStatus)
  - Implemented SubgroupHeader per draft-16 section 10.4.2 with three ID encoding modes
  - Unified OK responses: REQUEST_OK replaces PublishNamespaceOk, SubscribeNamespaceOk, TrackStatusOk
  - Unified error responses: REQUEST_ERROR replaces FetchError, PublishError, SubscribeError, SubscribeNamespaceError, PublishNamespaceError, TrackStatusError
  - SUBSCRIBE_UPDATE renamed to REQUEST_UPDATE; update propagation added for all message types
  - Unified request ID registry mapping request_id to handler type for correct response routing
  - Bitmask-based FetchObject serialization with delta encoding for sequential objects per draft-16 section 10.4.4
  - SUBSCRIBE_NAMESPACE uses a dedicated request stream; added Namespace and NamespaceDone messages
  - Relay implements draft-16 scheduling algorithm based on combined subscriber and publisher priorities
  - Removed synthetic Subscribe hack for Publish-based subscriptions
  - Renamed request_id field to max_request_id
  - Added client-js browser subscriber app and meet video conferencing demo app

## 0.9.0

### Minor Changes

- [#137](https://github.com/moqtail/moqtail/pull/137) [`19f19e7`](https://github.com/moqtail/moqtail/commit/19f19e71a1117d90be5d68c839adeb2b02cbc518) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Added publish_done message and corrected publish handler

## 0.8.2

### Patch Changes

- [`9f8dc10`](https://github.com/moqtail/moqtail/commit/9f8dc107cd815067ffc6f571db5b562834b2deaf) Thanks [@acbegen](https://github.com/acbegen)! - add cmsf catalog format and some cleanup

## 0.8.1

### Patch Changes

- [#141](https://github.com/moqtail/moqtail/pull/141) [`fc0c32f`](https://github.com/moqtail/moqtail/commit/fc0c32f280fa55c998da4b612438b9f276fc03b0) Thanks [@zafergurel](https://github.com/zafergurel)! - fixes the type of a variable in fetch.ts

## 0.8.0

### Minor Changes

- [#134](https://github.com/moqtail/moqtail/pull/134) [`03e3a62`](https://github.com/moqtail/moqtail/commit/03e3a629ed109010b27c15b0ad8d15a2bf3393cc) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Added the subscribenamespace-publish workflow

### Patch Changes

- [`c16fab7`](https://github.com/moqtail/moqtail/commit/c16fab77395de3ccd99c25b43ed6fd2754129d70) Thanks [@zafergurel](https://github.com/zafergurel)! - feat: add datagram draft-14 support, remove deprecated AkamaiOffset, update package description
  - feat(moqtail-rs, moqtail-ts): Add datagram draft-14 compatibility across both
    the Rust and TypeScript libraries. Updates datagram object parsing, datagram
    status handling, object model, and constants in both libs; also adjusts the
    relay's track handling and the TypeScript client/datagram stream accordingly.
  - refactor(moqtail-ts): Remove the deprecated AkamaiOffset utility class from
    the TypeScript library. ClockNormalizer is its replacement. Cleans up the
    export index and updates the README to reflect the removal.
  - chore(moqtail-rs): Update the moqtail-rs crate description in Cargo.toml.

## 0.7.0

### Minor Changes

- [#104](https://github.com/moqtail/moqtail/pull/104) [`a08c438`](https://github.com/moqtail/moqtail/commit/a08c4380f7a0abd25fcfa424ce3fbb5d90b4a977) Thanks [@zafergurel](https://github.com/zafergurel)! - Implements SWITCH message

## 0.6.0

### Minor Changes

- [#95](https://github.com/moqtail/moqtail/pull/95) [`5cd1a9e`](https://github.com/moqtail/moqtail/commit/5cd1a9ee3a04cb0a086c0873772d1ed8f85136d1) Thanks [@zafergurel](https://github.com/zafergurel)! - Add datagram support for publishing and subscribing to objects via QUIC datagrams

### Patch Changes

- [#78](https://github.com/moqtail/moqtail/pull/78) [`a32a2fc`](https://github.com/moqtail/moqtail/commit/a32a2fc2b59d2175e458d3cfbbe2cbd04e6579aa) Thanks [@zafergurel](https://github.com/zafergurel)! - revert the change for forward (number) change

## 0.5.1

### Patch Changes

- [#72](https://github.com/moqtail/moqtail/pull/72) [`af87c49`](https://github.com/moqtail/moqtail/commit/af87c49ee21cf255a0d47e218e793f58aa5ecadb) Thanks [@zafergurel](https://github.com/zafergurel)! - improved subscribe update handling

## 0.5.0

### Minor Changes

- [#69](https://github.com/moqtail/moqtail/pull/69) [`a22396a`](https://github.com/moqtail/moqtail/commit/a22396a9e891b2424a14b2ad18682473f890093d) Thanks [@acbegen](https://github.com/acbegen)! - integrated draft 14 changes

## 0.4.1

### Patch Changes

- [`92b8280`](https://github.com/moqtail/moqtail/commit/92b82804ed8dd981ceb138c6fedad7a6a69dc092) Thanks [@acbegen](https://github.com/acbegen)! - fix infinite loop case when client is destroyed

## 0.4.0

### Minor Changes

- [`6bfeaff`](https://github.com/moqtail/moqtail/commit/6bfeaff649ba665a85d2d61ef00289512eaf4f39) Thanks [@acbegen](https://github.com/acbegen)! - Handle errors in accepting unidirectional streams more gracefully

## 0.3.2

### Patch Changes

- [`d13be5e`](https://github.com/streaming-university/moqtail/commit/d13be5e2ca051d85c5230dcc3e94a5036c9bf08b) Thanks [@acbegen](https://github.com/acbegen)! - bump version to integrate the latest changes

## 0.3.1

### Patch Changes

- [#48](https://github.com/streaming-university/moqtail/pull/48) [`b89e901`](https://github.com/streaming-university/moqtail/commit/b89e901bcce0d70d5d07ae9baaccafefbe757550) Thanks [@LeventAksakal](https://github.com/LeventAksakal)! - Add documentation for most of the public facing library api
  - add api-extractor for document standardization
  - add type-doc for static site generation based off tsdocs
  - add documentation for the following items:
    - moqtail client, client options
    - track, track source and object cache
    - some items under ./model (e.g reason phrase, full track name ...)
  - update client-js library imports to use aliases

## 0.3.0

### Minor Changes

- [#33](https://github.com/streaming-university/moqtail/pull/33) [`0b59e15`](https://github.com/streaming-university/moqtail/commit/0b59e1582aca3eb307e75097b4a3716971dd523d) Thanks [@LeventAksakal](https://github.com/LeventAksakal)! - Refactor fetch message structure for better type safety and API design
  - **BREAKING**: Refactored `Fetch` class to use discriminated union for `typeAndProps`
    - Removed static factory methods (`Fetch.newStandAlone`, `Fetch.newRelative`, `Fetch.newAbsolute`)
    - Combined fetch type and properties into a single discriminated union field
    - Enables automatic TypeScript type narrowing based on fetch type
  - **NEW**: Implemented both publisher and subscriber functionality for fetch operations
  - **TODO**: Joining fetches require additional handling (partial implementation)
  - **PERFORMANCE**: Reduced test timeouts to improve CI pipeline speed
    - Telemetry tests: 5s → 1s timeouts
    - ControlStream tests: 3s → 1s timeouts
    - AkamaiOffset tests: mocked network calls instead of real ones
  - **CI**: Update husky hooks
    - Remove exec < /dev/tty from pre-commit and prepare-commit-msg for windows compatibility
  - **FIX**: Track alias map
    - Track alias map was using number for track alias instead of bigint
    - Update the structure and relevant parts of the code

  This change provides a more idiomatic and type-safe API for fetch operations while maintaining backward compatibility for serialization/deserialization.

- [#33](https://github.com/streaming-university/moqtail/pull/33) [`0b59e15`](https://github.com/streaming-university/moqtail/commit/0b59e1582aca3eb307e75097b4a3716971dd523d) Thanks [@LeventAksakal](https://github.com/LeventAksakal)! - Refactor client API and enhance subscribe request functionality
  - **BREAKING**: Updated subscribe request API for better control and lifecycle management
  - **NEW**: Added update functionality to subscribe requests
    - Allows modifying subscription parameters after initial request
    - Enables dynamic subscription management
  - **NEW**: Added cancel functionality to subscribe requests
    - Provides clean subscription cleanup
    - Prevents memory leaks from abandoned subscriptions
  - **PERFORMANCE**: Removed slow Akamai unit tests
    - Tests were causing CI pipeline delays
  - **API**: Improved client API ergonomics
    - Better type safety for subscribe operations
    - Remove redundant public identifiers (everything is public by default)
    - Use # for private members

  This change provides better control over subscription lifecycle

- [#46](https://github.com/streaming-university/moqtail/pull/46) [`74de932`](https://github.com/streaming-university/moqtail/commit/74de932bfd6d002b350eda1e09208ca39975d745) Thanks [@kerembkmz](https://github.com/kerembkmz)! - Media stream subscription management added with independent video/audio control.
