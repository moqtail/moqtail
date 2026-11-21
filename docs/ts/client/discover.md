---
description: Find the namespaces under a prefix and receive every track published under one with MOQtailClient.
---

# Discover namespaces and tracks

Use these requests when you don't know the names in advance: list the namespaces under a prefix, or have the relay send you every track published under one. You need a connected client: see [Connect to a relay](/ts/client/connect).

## List the namespaces under a prefix

`subscribeNamespace()` asks the relay for the namespaces that start with a prefix, now and later. They arrive as events, so attach the listeners first:

```ts
import { RequestError, Tuple } from 'moqtail'

client.on('peernamespace', (prefix, suffix) => {
  console.log('Found', prefix.toUtf8Path() + suffix.toUtf8Path())
})
client.on('peernamespacedone', (prefix, suffix) => {
  console.log('Gone', prefix.toUtf8Path() + suffix.toUtf8Path())
})

const namespaces = await client.subscribeNamespace(Tuple.fromUtf8Path('moqtail'))
if (namespaces.response instanceof RequestError) {
  throw new Error(`Refused: ${namespaces.response.reasonPhrase.phrase}`)
}
```

On `relay.moqtail.dev` this prints the public streams, such as `Found /moqtail/ch07`. `peernamespace` fires for each namespace that is announced, `peernamespacedone` for each one that is withdrawn. Both pass the prefix you subscribed with and the rest of the namespace after it, so you can tell several prefix subscriptions apart. To subscribe to a track in one of them, join the two:

```ts
import { FullTrackName } from 'moqtail'

function catalogOf(prefix: Tuple, suffix: Tuple): FullTrackName {
  return FullTrackName.tryNew(new Tuple([...prefix.fields, ...suffix.fields]), 'catalog')
}
```

This only lists namespaces: nothing is subscribed to. Call `namespaces.cancel()` to stop the updates.

## Receive every track under a prefix

`subscribeTracks()` asks the relay to push you every track under a prefix, the ones already there and the ones that start later. The client accepts each push and fires `peerpublish` with the PUBLISH message and a stream of the track's objects. This example, based on [`apps/meet`](https://github.com/moqtail/moqtail/blob/main/apps/meet/src/App.tsx), reads the video of everyone in a room:

```ts
import type { MoqtObject } from 'moqtail'

async function play(stream: ReadableStream<MoqtObject>): Promise<void> {
  const reader = stream.getReader()
  for (let read = await reader.read(); !read.done; read = await reader.read()) {
    console.log(read.value.fullTrackName.toString(), read.value.location)
  }
}

client.on('peerpublish', (publish, stream) => {
  const name = new TextDecoder().decode(publish.fullTrackName.name)
  if (name !== 'video') {
    void stream.cancel()
    return
  }
  console.log(`${publish.fullTrackName.namespace.toUtf8Path()} started video, request ${publish.requestId}`)
  void play(stream)
})
client.on('peerpublishdone', (publishDone, requestId) => {
  console.log(`Request ${requestId} ended with status ${publishDone.statusCode}`)
})

const tracks = await client.subscribeTracks(Tuple.fromUtf8Path('my-app/room-1'))
if (tracks.response instanceof RequestError) {
  throw new Error(`Refused: ${tracks.response.reasonPhrase.phrase}`)
}
```

- Only one reader can read a stream. With several `peerpublish` listeners, each checks the track first and only the one that wants it reads it.
- A stream nobody reads keeps its objects in memory. Cancel the ones you don't want: the client then stops reading their data.
- `peerpublishdone` passes the `requestId` of the PUBLISH that started the track. Its stream closes once the track's last data has arrived.
- `peerpublishskipped` means the relay has a track under your prefix but no stream left to push it on. Subscribe to that track directly if you need it.
- The client calls `acceptPushedTrack()` for each PUBLISH before it fires `peerpublish`. Don't call it yourself: a second call takes the track's objects away from the stream the event gave you.

The relay pushes only tracks their publisher pushed to it with `publish()`. A track that is only served on request, after `publishNamespace()`, is not pushed: find it with `subscribeNamespace()` and subscribe to it. The relay leaves out your own tracks and tracks you have already subscribed to. Once it has pushed as many tracks on one subscription as its `max_publish_streams` setting allows (see the [configuration file](/relay/reference/config)), it sends PUBLISH_SKIPPED for the next one, which fires `peerpublishskipped`, and pushes nothing more on that subscription.

Call `tracks.cancel()` to stop the pushes.

The relay refuses a second `subscribeNamespace()` whose prefix overlaps one you have, and the same for `subscribeTracks()`, with `PREFIX_OVERLAP`. The two kinds don't conflict with each other.

## More information

- [`MOQtailClient`](/ts/api/client/classes/MOQtailClient)
- [`ClientEvents`](/ts/api/client/interfaces/ClientEvents)
- [`Tuple`](/ts/api/model/classes/Tuple) and [`FullTrackName`](/ts/api/model/classes/FullTrackName)
- [`Publish`](/ts/api/model/classes/Publish), [`PublishDone`](/ts/api/model/classes/PublishDone) and [`PublishSkipped`](/ts/api/model/classes/PublishSkipped)
- [`RequestOk`](/ts/api/model/classes/RequestOk) and [`RequestError`](/ts/api/model/classes/RequestError)
