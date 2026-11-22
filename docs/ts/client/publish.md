---
description: Announce a namespace, register a track, push it to the relay and write its objects with MOQtailClient, then end it cleanly.
---

# Publish a track

A publisher announces a namespace, registers each track with the client, and writes the track's objects into a stream. The client sends them to whoever asks. This page follows the publisher in [`apps/client-js`](https://github.com/moqtail/moqtail/blob/main/apps/client-js/src/lib/publisher.ts), which publishes a camera as video tracks and a catalog.

You need a connected client: see [Connect to a relay](/ts/client/connect). The examples publish to a local relay at `moqt://localhost:4433`.

## Announce the namespace

```ts
import { isRequestError, Tuple } from 'moqtail'

const namespace = Tuple.fromUtf8Path('my-app/camera')
const announced = await client.publishNamespace(namespace)
if (isRequestError(announced)) {
  throw new Error(`Namespace refused: ${announced.reasonPhrase}`)
}
```

`publishNamespace()` returns the relay's `RequestOk` or its `RequestError`. The namespace stays announced until you call `unpublishNamespace()` or the session ends. While it is announced, the relay passes on SUBSCRIBE and FETCH requests for tracks in it to you.

## Register the track

A track's live objects come from a `ReadableStream` of `MoqtObject`s, wrapped in a `LiveTrackSource`. Keep the stream's controller: you write objects with it.

```ts
import { FullTrackName, LiveTrackSource, type MoqtObject, type Track } from 'moqtail'

const fullTrackName = FullTrackName.tryNew(namespace, 'video')

let controller!: ReadableStreamDefaultController<MoqtObject>
const objects = new ReadableStream<MoqtObject>({
  start: (c) => {
    controller = c
  },
})

const track: Track = {
  fullTrackName,
  trackSource: { live: new LiveTrackSource(objects) },
  publisherPriority: 1,
}
client.addOrUpdateTrack(track)
```

`addOrUpdateTrack()` gives the track a random track alias when it has none. `publisherPriority` goes from 0, the highest, to 255, the lowest.

From now on the client answers a SUBSCRIBE for this track and sends it the objects you write. It refuses tracks you haven't registered with `DOES_NOT_EXIST`. If that is all you need, skip to [Write objects](#write-objects): the relay asks for the track when a subscriber does.

## Push it to the relay

To have the relay take the track before anyone asks for it, publish it:

```ts
const published = await client.publish(fullTrackName, true, track.trackAlias!)
if (isRequestError(published)) {
  throw new Error(`Track refused: ${published.reasonPhrase}`)
}
```

The arguments are the track, the forward state, and the alias `addOrUpdateTrack()` gave it. Once the relay accepts, the client sends it every object you write. A track you push also reaches subscribers that [ask for every track under a prefix](/ts/client/discover#receive-every-track-under-a-prefix).

With `false` as the forward state, the client holds back objects until the relay asks for them. `forwardstatechange` tells you when that changes:

```ts
client.on('forwardstatechange', (requestId, forward) => {
  if (requestId === published.requestId) console.log(forward ? 'Sending' : 'Paused')
})
```

The relay can also ask you for things. The client answers these requests itself, and the events let you see them: `peersubscribenamespace` when the relay subscribes to a namespace prefix you publish under, and `peersubscribetracks` when it asks for every track under a prefix, which the client then pushes to it.

## Write objects

Each object needs a location: a group ID and an object ID within the group. A group is where a subscriber can start, so a video track starts a new group at each key frame:

```ts
import { LOCProperties, Location, MoqtObject, ObjectForwardingPreference } from 'moqtail'

let group = 0n
let object = 0n

function send(payload: Uint8Array, isKey: boolean): void {
  if (isKey) {
    group++
    object = 0n
  }
  const location = new Location(group, object)
  object++

  const properties = new LOCProperties()
    .addTimestamp(Date.now() * 1000)
    .addVideoFrameMarking(isKey)
    .build()

  controller.enqueue(
    MoqtObject.newWithPayload(fullTrackName, location, 1, ObjectForwardingPreference.Subgroup, 0n, properties, payload),
  )
}
```

The arguments of `MoqtObject.newWithPayload()` are the track, the location, the publisher priority, the forwarding preference, the subgroup ID, the object properties (or `null`) and the payload. `LOCProperties` builds the LOC properties: here the capture time in microseconds since the Unix epoch, and whether the frame is a key frame.

Call `send()` with each encoded frame, for example from a `VideoEncoder`:

```ts
const encoder = new VideoEncoder({
  output: (chunk) => {
    const payload = new Uint8Array(chunk.byteLength)
    chunk.copyTo(payload)
    send(payload, chunk.type === 'key')
  },
  error: (error) => console.error('Encoder failed:', error),
})
```

The client opens a data stream for each group and closes it when the next group starts. It sends only objects you write after a subscription starts or the relay accepts the push. Objects with `ObjectForwardingPreference.Datagram` are not sent: to send datagrams, connect with `enableDatagrams: true` and write them with `client.createDatagramSender(trackAlias)`.

A player also needs a catalog that describes your tracks. [Catalogs and packaging](/guide/concepts/media) explains what goes in it.

## Serve past objects

A FETCH asks for objects already published. The client answers it from the track's past source, and refuses it with `NOT_SUPPORTED` for a track without one. To serve both, register the track with a `HybridTrackSource` instead, and add each object to its cache as you write it:

```ts
import { HybridTrackSource, RingBufferObjectCache } from 'moqtail'

const cache = new RingBufferObjectCache(300)
const track: Track = {
  fullTrackName,
  trackSource: new HybridTrackSource(cache, objects),
  publisherPriority: 1,
}

function write(obj: MoqtObject): void {
  cache.add(obj)
  controller.enqueue(obj)
}
```

`RingBufferObjectCache` keeps the latest objects, 100 unless you give another number. `MemoryObjectCache` keeps all of them. For a recording with no live part, use `{ past: new StaticTrackSource(cache) }` as the track source.

## Stop publishing

Stop the encoder and close the stream, end the push, then withdraw the track and the namespace:

```ts
encoder.close()
controller.close()
await client.publishDone(published.requestId)
client.removeTrack(track)
await client.unpublishNamespace(namespace)
await client.disconnect()
```

Closing the stream ends every subscription the client answered for the track, with PUBLISH_DONE and `TRACK_ENDED`. A push needs `publishDone()` too. It reports `TRACK_ENDED` unless you give another status code. `removeTrack()` only stops the client from answering new requests for the track.

## More information

- [`MOQtailClient`](/ts/api/client/classes/MOQtailClient)
- [`Track`](/ts/api/client/type-aliases/Track) and [`TrackSource`](/ts/api/client/interfaces/TrackSource)
- [`LiveTrackSource`](/ts/api/client/classes/LiveTrackSource), [`StaticTrackSource`](/ts/api/client/classes/StaticTrackSource) and [`HybridTrackSource`](/ts/api/client/classes/HybridTrackSource)
- [`RingBufferObjectCache`](/ts/api/client/classes/RingBufferObjectCache) and [`MemoryObjectCache`](/ts/api/client/classes/MemoryObjectCache)
- [`MoqtObject`](/ts/api/model/classes/MoqtObject) and [`Location`](/ts/api/model/classes/Location)
- [`LOCProperties`](/ts/api/model/classes/LOCProperties)
- [`ObjectForwardingPreference`](/ts/api/model/enumerations/ObjectForwardingPreference)
- [`PublishDoneStatusCode`](/ts/api/model/enumerations/PublishDoneStatusCode)
