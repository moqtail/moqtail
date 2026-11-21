---
description: Subscribe to a track's live objects, fetch objects already published, and update, switch or end a subscription with MOQtailClient.
---

# Subscribe and fetch

Subscribe to get a track's objects as they are published. Fetch to get objects that are already published. Both need a connected client: see [Connect to a relay](/ts/client/connect).

## Name the track

A track is named by its namespace and its track name, together a `FullTrackName`:

```ts
import { FullTrackName } from 'moqtail'

const fullTrackName = FullTrackName.tryNew('moqtail/ch07', 'catalog')
```

The namespace is a `/`-separated string or a `Tuple`, the name a string or bytes. `tryNew` throws a `TrackNameError` when the namespace has more than 32 fields or the whole name is longer than 4096 bytes. [MOQT in brief](/guide/concepts/moqt) explains namespaces, tracks, groups and objects.

## Subscribe

```ts
import { GroupOrder, LocationFilter, LocationFilterType, RequestError } from 'moqtail'

const subscription = await client.subscribe({
  fullTrackName,
  priority: 128,
  groupOrder: GroupOrder.Original,
  locationFilter: new LocationFilter(LocationFilterType.NextObject),
})
if (subscription instanceof RequestError) {
  throw new Error(`Subscribe refused: ${subscription.reasonPhrase.phrase}`)
}
```

When the relay refuses, `subscribe()` returns its `RequestError`, with an `errorCode` and a `reasonPhrase`. When it accepts, you get:

- `requestId`, which names the subscription in later calls.
- `subscribeOk`, the relay's SUBSCRIBE_OK, with the track alias and parameters.
- `stream`, a `ReadableStream` of `MoqtObject`s.
- `done`, a promise for the PUBLISH_DONE that ends the subscription.

`subscribe()` throws when the client is not connected or the session fails. A session failure also closes the client.

The options:

| Option               | What it does                                                                                                               |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `fullTrackName`      | The track.                                                                                                                 |
| `priority`           | An integer from 0, the highest, to 255, the lowest. Other values are rounded and clamped.                                  |
| `groupOrder`         | `Original` keeps the publisher's order, `Ascending` and `Descending` ask for one.                                          |
| `locationFilter`     | Which objects to deliver: see [Choose where to start](#choose-where-to-start). Without it, the subscription is unfiltered. |
| `forward`            | `false` sets up the subscription without delivering objects yet. Left out, objects are delivered.                          |
| `parameters`         | More message parameters, such as a [fill](#start-at-the-beginning-of-the-current-group).                                   |
| `ordering`           | Overrides the client's `ordering`: see [Receive objects in order](#receive-objects-in-order).                              |
| `onEndOfRange`       | Called for each End of Range marker on a fill.                                                                             |
| `earlyDiscardPolicy` | `{ subgroupReceiveTimeout }`: drops a data stream that has not finished within that many ms.                               |

To see where the track is now, read the Largest Object from the SUBSCRIBE_OK:

```ts
import { MessageParameter } from 'moqtail'

const largest = subscription.subscribeOk.parameters.find(MessageParameter.isLargestObject)
console.log('Track is at', largest?.location)
```

## Read the objects

```ts
const reader = subscription.stream.getReader()
for (let read = await reader.read(); !read.done; read = await reader.read()) {
  const object = read.value
  // End of Group and End of Track objects carry a status instead of a payload
  if (!object.payload) continue
  console.log(`group ${object.location.group} object ${object.location.object}: ${object.payload.byteLength} bytes`)
}
```

Each object has its `location` (group and object ID), its `payload`, its `properties` and its `objectStatus`. Objects come in the order they arrive, which across several data streams is not always location order.

The properties are key-value pairs. `findProperty` reads one, such as the LOC Timestamp:

```ts
import { findProperty, Timestamp } from 'moqtail'

const sentAt = findProperty(object.properties, Timestamp)?.timestamp
```

The stream closes when the publisher has ended the track and all its data has arrived. `done` resolves then, with the PUBLISH_DONE, or with `undefined` when the subscription ended without one. It never rejects. If the session ends first, `done` resolves but the stream stays open, so stop reading on `sessionterminated` too.

```ts
void subscription.done.then((publishDone) => {
  console.log('Subscription over', publishDone?.statusCode, publishDone?.errorReason.phrase)
})
```

## Choose where to start

`locationFilter` takes a `LocationFilter`. Its type says which fields follow:

| Filter                                                                         | Delivers                                                             |
| ------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| `new LocationFilter(LocationFilterType.NextObject)`                            | From the object after the Largest Object                             |
| `new LocationFilter(LocationFilterType.RelativeStart, 0n)`                     | From the next group. `1n` is the current group, `2n` the one before. |
| `new LocationFilter(LocationFilterType.AbsoluteStart, 100n, 0n)`               | From group 100, object 0                                             |
| `new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 100n, 0n, 120n)` | From group 100, object 0, to the end of group 120                    |
| `new LocationFilter(LocationFilterType.AbsoluteRange, 100n, 0n, 120n, 4n)`     | From group 100, object 0, to group 120, object 4, inclusive          |

A start in the future waits for that object. The constructor throws a `ProtocolViolationError` when a field is missing or the end is before the start.

### Start at the beginning of the current group

A decoder usually needs a group from its first object, but `NextObject` starts in the middle of one. To also get the objects of the current group that are already published, add a fill:

```ts
import { FillParameters } from 'moqtail'

const subscription = await client.subscribe({
  fullTrackName,
  priority: 128,
  groupOrder: GroupOrder.Original,
  locationFilter: new LocationFilter(LocationFilterType.NextObject),
  parameters: [new FillParameters([new LocationFilter(LocationFilterType.RelativeStart, 1n)])],
})
```

The publisher sends the fill on a stream of its own, and its objects arrive on the same `stream` as the live ones. With [ordering](#receive-objects-in-order) on, they come in location order with them.

## Change or end a subscription

`subscribeUpdate()` changes a running subscription. What you leave out stays as it is:

```ts
await client.subscribeUpdate({ subscriptionRequestId: subscription.requestId, priority: 200 })
await client.subscribeUpdate({ subscriptionRequestId: subscription.requestId, forward: false })
```

`forward: false` pauses delivery and `forward: true` resumes it. `locationFilter` replaces the filter.

To end a subscription, stop reading and unsubscribe:

```ts
await reader.cancel()
await client.unsubscribe(subscription.requestId)
```

You need both. Cancelling the stream does not tell the relay, and `unsubscribe()` does not close the stream. `unsubscribe()` resolves `done` with `undefined`, and calling it again does nothing.

## Fetch past objects

`fetch()` takes the same track, priority and group order, and a filter for the objects you want. A filter without an end stops at the Largest Object, and no filter at all fetches the whole track up to it.

```ts
const fetched = await client.fetch({
  fullTrackName,
  priority: 128,
  groupOrder: GroupOrder.Original,
  locationFilter: new LocationFilter(LocationFilterType.RelativeStart, 2n),
})
if (fetched instanceof RequestError) {
  throw new Error(`Fetch refused: ${fetched.reasonPhrase.phrase}`)
}
console.log('Fetch ends at', fetched.fetchOk.endLocation, 'end of track:', fetched.fetchOk.endOfTrack)

const fetchReader = fetched.stream.getReader()
for (let read = await fetchReader.read(); !read.done; read = await fetchReader.read()) {
  console.log(read.value.location)
}
```

`RelativeStart` with `2n` fetches the current group and the one before it. The stream closes after the last object. A range the publisher has nothing for is refused with a `RequestError`.

Where a range of objects is missing, the publisher can say so with an End of Range marker. Pass `onEndOfRange: (kind, location) => …` to hear about them: every location up to `location` does not exist, is unknown or timed out, as `kind` says.

To stop a fetch early, cancel it and stop reading:

```ts
await client.fetchCancel(fetched.requestId)
await fetchReader.cancel()
```

## Receive objects in order

Without ordering, objects come as they arrive. With it, the client releases each request's objects in location order and waits a little for one that is missing. Set it for every request on the client, or per request:

```ts
const client = new MOQtailClient({
  url: 'moqt://relay.moqtail.dev',
  ordering: { window: { durationMs: 150 } },
})
client.on('orderinggap', (requestId, from, to) => {
  console.warn(`Request ${requestId} skipped ${from.group}:${from.object} up to ${to.group}:${to.object}`)
})
```

- `window` is how long an object may wait for the one before it (`durationMs`), and how many may wait (`objects`). When either runs out, the client releases what it holds and reports the skipped locations with `orderinggap`.
- `onWindowExceeded` is what happens to an object that arrives after the client stopped waiting for it: `'flush'` (the default) still delivers it, `'drop'` drops it.
- `sparseObjectIds` and `sparseGroupIds` say IDs may skip without the publisher marking the gap.

`ordering: 'none'` on a subscription or fetch turns it off for that request. A request with `GroupOrder.Descending` is never ordered.

Ordering also decides where datagrams go. A publisher may send objects as QUIC datagrams instead of on streams, and the client reads them only with `enableDatagrams: true`. An ordered subscription gets them on its `stream`. On an unordered one, read them with `client.subscribeToTrackDatagrams(subscription.subscribeOk.trackAlias, (object) => …)`.

## Switch to another track

`switch()` replaces a subscription with one to another track, such as another rendition, while you keep reading the same `stream`. With `SwitchMode.Soft`, the old track keeps delivering up to the group where the new one starts, so nothing is missing between them. `SwitchMode.Hard` stops the old track at once. The relay must support SWITCH_FROM.

```ts
import { SwitchMode } from 'moqtail'

const switched = await client.switch({
  switchFromRequestId: subscription.requestId,
  switchMode: SwitchMode.Soft,
  newSubscribeOptions: {
    fullTrackName: FullTrackName.tryNew('my-app/live', 'video-1080p'),
    priority: 128,
    groupOrder: GroupOrder.Original,
    locationFilter: new LocationFilter(LocationFilterType.NextObject),
  },
})
```

The result is a `RequestError` or a new `SubscribeResult`, whose `stream` and `done` are the original subscription's.

## More information

- [`MOQtailClient`](/ts/api/client/classes/MOQtailClient)
- [`SubscribeOptions`](/ts/api/client/type-aliases/SubscribeOptions), [`SubscribeResult`](/ts/api/client/interfaces/SubscribeResult) and [`SubscribeUpdateOptions`](/ts/api/client/type-aliases/SubscribeUpdateOptions)
- [`FetchOptions`](/ts/api/client/type-aliases/FetchOptions) and [`FetchResult`](/ts/api/client/interfaces/FetchResult)
- [`SwitchOptions`](/ts/api/client/type-aliases/SwitchOptions)
- [`ObjectOrdering`](/ts/api/client/interfaces/ObjectOrdering)
- [`FullTrackName`](/ts/api/model/classes/FullTrackName), [`LocationFilter`](/ts/api/model/classes/LocationFilter) and [`LocationFilterType`](/ts/api/model/enumerations/LocationFilterType)
- [`MoqtObject`](/ts/api/model/classes/MoqtObject)
- [`FillParameters`](/ts/api/model/classes/FillParameters)
- [`RequestError`](/ts/api/model/classes/RequestError)
