/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { RecvStream } from '@/client/data_stream'
import { getHandlerForControlMessage, getHandlerForRequestStreamMessage } from '@/client/handler/handler'
import { FetchRequest } from '@/client/request/fetch'
import { SubscribeRequest } from '@/client/request/subscribe'
import { RequestStream } from '@/client/request_stream'
import { PeerStreamResetError, streamResetReason } from '@/client/util/stream_reset'
import type { EndOfRangeHandler } from '@/client/types'
import type { Location } from '@/model/common/location'
import { GroupOrder } from '@/model/constants'
import { isFirst } from '@/model/messages'
import type { Message } from '@/model/messages'
import { SubgroupHeaderType } from '@/model/data/constant'
import { FetchHeader } from '@/model/data/fetch_header'
import { FetchObject } from '@/model/data/fetch_object'
import type { FullTrackName } from '@/model/data/full_track_name'
import type { Header } from '@/model/data/header'
import { MoqtObject } from '@/model/data/object'
import { SubgroupHeader } from '@/model/data/subgroup_header'
import { SubgroupObject } from '@/model/data/subgroup_object'
import { MOQtailError, ProtocolViolationError } from '@/model/error/error'
import { StreamResetCode } from '@/model/error/stream_reset'
import { logger } from '@/util/logger'
import { waitForTrackAlias } from './aliases'
import { clientRequestId } from './request_streams'
import { closeSession } from './session'
import type { Session } from './session'
import { completeSubscription } from './subscribe'

type ObjectReader = ReadableStreamDefaultReader<FetchObject | SubgroupObject>

/** Reads the shared control stream; every request type has its own bidi stream. */
export async function readControlStream(session: Session): Promise<void> {
  const { client } = session

  try {
    const reader = client.controlStream.stream.getReader()

    while (true) {
      const { done, value: msg } = await reader.read()
      if (done) throw new MOQtailError('WebTransport session is terminated')

      const handler = getHandlerForControlMessage(msg)
      if (!handler) {
        // Strictly a PROTOCOL_VIOLATION, but the relay still pushes a few messages here
        // (PUBLISH_DONE, REQUEST_UPDATE fan-out). Warn rather than tear the session down
        // until that is cleaned up.
        logger.warn('MOQtailClient', `${msg.kind} on the control stream`)
        continue
      }

      await handler(client, msg as Extract<Message, { kind: 'goAway' }>)
    }
  } catch (error) {
    // Nothing awaits this loop: the error ends the session here instead of surfacing unhandled
    if (!session.closing(error)) logger.error('MOQtailClient', 'control stream failed', error)

    await closeSession(session, error)
  }
}

export async function acceptIncomingUniStreams(session: Session): Promise<void> {
  try {
    while (true) {
      const { done, value: stream } = await session.incomingUniStreams.read()
      if (done) return

      // Streams are served concurrently, so each rejection is handled here or it surfaces
      // unhandled. A peer reset is an ordinary end for a data stream (the subscription was
      // dropped, TooFarBehind, a delivery timeout), so it is reported, not escalated.
      receiveDataStream(session, stream).catch((error) => {
        if (error instanceof PeerStreamResetError || session.closing(error)) {
          logger.debug('MOQtailClient', `data stream ended: ${error.message}`)
          return
        }

        logger.error('MOQtailClient', 'handleRecvStreams error', error)
      })
    }
  } catch (error) {
    // A failed read leaves the reader errored: the session is gone
    if (!session.closing(error)) logger.error('MOQtailClient', 'acceptIncomingUniStreams error', error)
  }
}

export async function acceptIncomingBiStreams(session: Session): Promise<void> {
  const { client } = session
  const reader = client.webTransport.incomingBidirectionalStreams.getReader()

  try {
    while (true) {
      const { value: biStream, done } = await reader.read()
      if (done) return

      serveRequestStream(session, new RequestStream(biStream, client)).catch((error) => {
        if (!session.closing(error)) logger.error('MOQtailClient', 'incoming request stream failed', error)
      })
    }
  } catch (error) {
    if (!session.closing(error)) logger.error('MOQtailClient', 'acceptIncomingBiStreams error', error)
  }
}

/**
 * Serves one peer-opened request stream: the first message must be a `First`-marked type,
 * its handler answers on this same stream, and follow-ups are read until the peer closes
 * it. That close is the peer's cancellation, so it tears down whatever the first message
 * started.
 */
async function serveRequestStream(session: Session, requestStream: RequestStream): Promise<void> {
  const { client } = session
  const first = await requestStream.next()
  if (!first) return

  if (!isFirst(first.kind)) {
    logger.warn('MOQtailClient', `${first.kind} may not open a request stream; resetting it`)
    await requestStream.reset(StreamResetCode.InternalError)
    return
  }

  // The first message names the request; everything later on this stream belongs to it.
  const openingRequestId = (first as { requestId: bigint }).requestId
  requestStream.openingType = first.kind

  try {
    let msg: Message | undefined = first

    while (msg) {
      const handler = getHandlerForRequestStreamMessage(msg)
      if (!handler) {
        throw new ProtocolViolationError('MOQtailClient', `No handler for ${msg.kind} on a request stream`)
      }

      await handler(client, msg, requestStream, openingRequestId)
      msg = await requestStream.next()
    }
  } catch (error) {
    if (!session.closing(error)) logger.error('MOQtailClient', 'incoming request stream failed', error)
  } finally {
    // The peer closed or reset the stream: cancel whatever it was serving. Every
    // First-marked type carries a request id, but the union as a whole does not.
    const requestId = 'requestId' in first ? first.requestId : undefined
    if (requestId !== undefined) {
      client.publications.get(requestId)?.cancel()
      client.publications.delete(requestId)
    }

    await requestStream.close()
  }
}

async function receiveDataStream(session: Session, incomingUniStream: ReadableStream): Promise<void> {
  const { client } = session

  session.ensureActive()

  const recvStream = await RecvStream.new(
    incomingUniStream,
    client.dataStreamTimeoutMs,
    client,
    (header) => fetchGroupOrder(session, header),
    (header) => streamTrackName(session, header),
  )
  const { header } = recvStream

  if (header instanceof SubgroupHeader) {
    await receiveSubgroupStream(session, recvStream, header)
    return
  }

  // The header names the fetch by the id it was issued under, which a migration may have
  // moved on from.
  const request = client.requests.get(clientRequestId(session, header.requestId))
  if (request instanceof FetchRequest) {
    await receiveFetchStream(recvStream, request)
    return
  }

  // A fill fetch stream: the same header, but named by the SUBSCRIBE or REQUEST_UPDATE that
  // asked for the fill. Its objects belong on that subscription's stream, alongside the live ones.
  if (request instanceof SubscribeRequest) {
    await receiveFillStream(session, recvStream, request)
    return
  }

  throw new ProtocolViolationError('MOQtailClient', 'No request for received request id')
}

async function receiveFetchStream(recvStream: RecvStream, request: FetchRequest): Promise<void> {
  const reader = recvStream.stream.getReader()
  let previous: Location | undefined

  try {
    for await (const moqtObject of readFetchObjects(reader, request.message.fullTrackName, request.onEndOfRange)) {
      // The consumer cancelled its stream, a seek for one: the rest of this one goes nowhere
      if (!request.objects.open) {
        await recvStream.stopSending(StreamResetCode.Cancelled)
        break
      }

      if (request.orderer) request.orderer.push(moqtObject, previous)
      else request.objects.enqueue(moqtObject)
      previous = moqtObject.location
    }

    // The FETCH_OK handler removes the request, not the end of its data stream
    request.orderer?.flush()
    request.objects.close()
  } finally {
    reader.releaseLock()
  }
}

async function receiveFillStream(session: Session, recvStream: RecvStream, request: SubscribeRequest): Promise<void> {
  const reader = recvStream.stream.getReader()

  // The publisher counts it in Stream Count like any stream it opened, so the subscriber
  // has to count it too or the two never agree.
  request.streamsAccepted++
  request.fillOpened = true
  request.openFills++
  request.orderer?.suspendWindow('fill')

  let previous: Location | undefined
  let force = false

  try {
    for await (const moqtObject of readFetchObjects(reader, request.fullTrackName, request.onEndOfRange)) {
      // Unsubscribing is the consumer's to do; this stream just stops
      if (!request.manager.objects.open) {
        await recvStream.stopSending(StreamResetCode.Cancelled)
        break
      }

      if (!request.manager.deliver(request, moqtObject, previous)) {
        await recvStream.stopSending(StreamResetCode.Cancelled)
        force = true
        break
      }

      previous = moqtObject.location
    }
  } finally {
    reader.releaseLock()
    // Fills may overlap (moqt-22 §3.4): the window resumes once the last one ends
    request.openFills--
    if (request.openFills === 0) request.orderer?.resumeWindow('fill')
  }

  // It counts towards the subscription's streams like any other, so it can equally be the
  // one that completes it.
  completeSubscription(session, request, force)
}

async function receiveSubgroupStream(session: Session, recvStream: RecvStream, header: SubgroupHeader): Promise<void> {
  const { client } = session
  // A publisher opens a subscription's data streams as soon as it has written the control
  // message naming their alias, and nothing orders the two streams: the data can arrive
  // first. The holder is read back from the table rather than handed over by the wait: a
  // SubscribeRequest is itself a thenable, so a promise resolved with one adopts it.
  let subscription = client.subscriptions.get(header.trackAlias)
  if (!subscription) {
    await waitForTrackAlias(session, header.trackAlias)
    subscription = client.subscriptions.get(header.trackAlias)
  }

  if (!subscription) throw new ProtocolViolationError('MOQtailClient', 'No subscription for received track alias')

  const reader = recvStream.stream.getReader()

  subscription.streamsAccepted++

  // The request that owns this stream names its track. Resolving the alias per object
  // would break once the alias mapping is torn down: a switch retires the request it
  // switched from while its subgroup streams are still draining.
  const fullTrackName: FullTrackName = subscription.fullTrackName
  let firstObjectId: bigint | undefined
  let lastObjectId: bigint | undefined
  let force = false

  const discardPolicy = subscription.earlyDiscardPolicy ?? session.earlyDiscardPolicy
  const timeoutId =
    discardPolicy?.subgroupReceiveTimeout === undefined
      ? undefined
      : setTimeout(() => {
          reader.cancel(streamResetReason(StreamResetCode.DeliveryTimeout)).catch(() => {})
        }, discardPolicy.subgroupReceiveTimeout)

  try {
    while (true) {
      const { done, value: next } = await reader.read()
      if (done) {
        // A clean end of a stream carrying the End of Group: its last object ends the group
        if (lastObjectId !== undefined && SubgroupHeaderType.containsEndOfGroup(header.type)) {
          subscription.orderer?.endGroup(header.groupId, lastObjectId)
        }

        break
      }
      if (!next) continue
      if (!(next instanceof SubgroupObject)) {
        throw new ProtocolViolationError('MOQtailClient', 'Received fetch object after subgroup header')
      }

      lastObjectId = next.objectId
      firstObjectId ??= next.objectId

      const moqtObject = MoqtObject.fromSubgroupObject(
        next,
        header.groupId,
        header.publisherPriority,
        subgroupIdOf(header, firstObjectId),
        fullTrackName,
      )

      if (!subscription.largestLocation || subscription.largestLocation.compare(moqtObject.location) === -1) {
        subscription.largestLocation = moqtObject.location
      }

      // Unsubscribing is the consumer's to do; this stream just stops
      if (!subscription.manager.objects.open) {
        await recvStream.stopSending(StreamResetCode.Cancelled)
        break
      }
      if (!subscription.manager.deliver(subscription, moqtObject)) {
        force = true
        await recvStream.stopSending(StreamResetCode.Cancelled)
        break
      }
    }
  } finally {
    clearTimeout(timeoutId)
  }

  completeSubscription(session, subscription, force)
}

function subgroupIdOf(header: SubgroupHeader, firstObjectId: bigint): bigint | null {
  if (SubgroupHeaderType.isSubgroupIdZero(header.type)) return 0n
  if (SubgroupHeaderType.isSubgroupIdFirstObjectId(header.type)) return firstObjectId
  if (SubgroupHeaderType.hasExplicitSubgroupId(header.type)) return header.subgroupId ?? null

  return null
}

/** The group order a fetch stream's objects are delta-coded in: its FETCH's. */
function fetchGroupOrder(session: Session, header: Header): GroupOrder {
  if (!(header instanceof FetchHeader)) return GroupOrder.Original

  const request = session.client.requests.get(clientRequestId(session, header.requestId))
  if (!(request instanceof FetchRequest)) return GroupOrder.Original

  return request.message.parameters.groupOrder ?? GroupOrder.Original
}

/** A fetch stream is named by its FETCH, or by the SUBSCRIBE it fills. */
function streamTrackName(session: Session, header: Header): FullTrackName | undefined {
  const { client } = session
  if (header instanceof SubgroupHeader) return client.aliasFullTrackNameMap.get(header.trackAlias)

  const request = client.requests.get(clientRequestId(session, header.requestId))
  if (request instanceof FetchRequest) return request.message.fullTrackName

  return request instanceof SubscribeRequest ? request.fullTrackName : undefined
}

/** The objects on a fetch data stream. End-of-range markers carry no Object, so they go to `onEndOfRange`. */
async function* readFetchObjects(
  reader: ObjectReader,
  fullTrackName: FullTrackName,
  onEndOfRange: EndOfRangeHandler | undefined,
): AsyncGenerator<MoqtObject> {
  while (true) {
    const { done, value } = await reader.read()
    if (done) return
    if (!value) continue
    if (!(value instanceof FetchObject)) {
      throw new ProtocolViolationError('MOQtailClient', 'Received subgroup object after fetch header')
    }

    if (value.kind === 'object') {
      yield MoqtObject.fromFetchObject(value, fullTrackName)
      continue
    }

    // A handler that throws must not end the stream the marker arrived on
    try {
      onEndOfRange?.(value.endOfRange!, value.location)
    } catch (handlerError) {
      logger.warn('MOQtailClient', 'End of Range handler error:', handlerError)
    }
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { SubscribePublication } = await import('@/client/publication/subscribe')
  const { connected, fetchStreamBytes, objectAt, openedStream, restoreWebTransport, subscribeOk } =
    await import('@/client/testing')
  const { LiveTrackSource } = await import('@/client/track/content_source')
  const { streamResetCodeOf } = await import('@/client/util/stream_reset')
  const { ByteBuffer } = await import('@/model/common/byte_buffer')
  const { Location } = await import('@/model/common/location')
  const { LocationFilterType, PublishDoneStatusCode } = await import('@/model/constants')
  const { FetchHeaderType, ObjectForwardingPreference } = await import('@/model/data/constant')
  const { EndOfRangeKind } = await import('@/model/data/fetch_object')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { Header } = await import('@/model/data/header')
  const { encodeMessage, requestOk } = await import('@/model/messages')
  const { locationFilter } = await import('@/model/parameters')
  type EndOfRangeKind = import('@/model/data/fetch_object').EndOfRangeKind
  type GoAway = import('@/model/messages').GoAway
  type Subscribe = import('@/model/messages').Subscribe
  type SubscribeResult = import('@/client/types').SubscribeResult

  describe('MOQtailClient incoming streams', () => {
    restoreWebTransport()

    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('answers a peer-opened request stream on that stream', async () => {
      const { client, transport } = await connected()

      const incoming = transport.openIncomingBiStream()
      const subscribe: Subscribe = {
        kind: 'subscribe',
        requestId: 4n,
        fullTrackName: ftn,
        parameters: { locationFilter: locationFilter(LocationFilterType.NextObject) },
      }
      incoming.respond(subscribe)

      // No such track is registered, so the refusal comes back here rather than on the
      // control stream.
      await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))
      expect(incoming.messages[0]).toMatchObject({ kind: 'requestError' })
      expect(transport.sentChunks).toHaveLength(1)

      await client.disconnect()
    })

    it('applies a REQUEST_UPDATE on a SUBSCRIBE stream', async () => {
      const { client, transport } = await connected()

      client.addOrUpdateTrack({
        fullTrackName: ftn,
        trackSource: { live: new LiveTrackSource(new ReadableStream<MoqtObject>()) },
        publisherPriority: 0,
        trackAlias: 7n,
      })

      const incoming = transport.openIncomingBiStream()
      incoming.respond({
        kind: 'subscribe',
        requestId: 4n,
        fullTrackName: ftn,
        parameters: { locationFilter: locationFilter(LocationFilterType.NextObject) },
      })
      await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))
      expect(incoming.messages[0]).toMatchObject({ kind: 'subscribeOk' })

      incoming.respond({ kind: 'requestUpdate', requestId: 4n, parameters: { subscriberPriority: 200 } })
      await vi.waitFor(() => expect(incoming.messages).toHaveLength(2))
      expect(incoming.messages[1]).toMatchObject({ kind: 'requestOk' })
      expect(client.publications.get(4n)).toBeInstanceOf(SubscribePublication)

      await client.disconnect()
    })

    it('closes the session on a second GOAWAY on the control stream', async () => {
      const { client, control } = await connected()
      const seen: GoAway[] = []
      const alsoSeen: GoAway[] = []
      const terminated: unknown[] = []

      client.on('goaway', (msg) => seen.push(msg))
      client.on('goaway', (msg) => alsoSeen.push(msg))
      client.on('sessionterminated', (reason) => terminated.push(reason))

      control.enqueue(encodeMessage({ kind: 'goAway', newSessionUri: 'https://elsewhere.example', timeout: 5000n }))
      await vi.waitFor(() => expect(seen).toHaveLength(1))
      expect(seen[0]!.newSessionUri).toBe('https://elsewhere.example')
      expect(seen[0]!.timeout).toBe(5000n)
      expect(client.goawayReceived).toBe(true)

      // §10.4: more than one GOAWAY on the control stream is a protocol violation.
      control.enqueue(encodeMessage({ kind: 'goAway', timeout: 0n }))
      await vi.waitFor(() => expect(terminated).toHaveLength(1))
      expect(seen).toHaveLength(1)
      expect(alsoSeen).toEqual(seen)
    })

    it('stops the data streams of a subscription whose consumer cancelled, and still completes it', async () => {
      const { client, transport } = await connected()
      const errors = vi.spyOn(logger, 'error')
      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      subscribeStream.respond(subscribeOk(7n, {}, []))

      const subscribed = (await subscribing) as SubscribeResult
      await subscribed.stream.cancel()

      const bytes = new ByteBuffer()
      bytes.putBytes(Header.newSubgroup(0x10, 7n, 1n, undefined, 0).serialize().toUint8Array())
      bytes.putBytes(
        SubgroupObject.newWithPayload(0, null, new Uint8Array([1]))
          .serialize(undefined)
          .toUint8Array(),
      )
      transport.openIncomingUniStream(bytes.toUint8Array())
      await vi.waitFor(() => expect(transport.uniCancelReasons).toHaveLength(1))
      expect(streamResetCodeOf(transport.uniCancelReasons[0])).toBe(StreamResetCode.Cancelled)

      // Its stream can't be closed any more, which must not stop the request being forgotten
      subscribeStream.respond({
        kind: 'publishDone',
        statusCode: PublishDoneStatusCode.TrackEnded,
        streamCount: 1n,
        errorReason: '',
      })
      expect((await subscribed.done)?.statusCode).toBe(PublishDoneStatusCode.TrackEnded)
      expect(client.requests.has(subscribed.requestId)).toBe(false)
      expect(errors).not.toHaveBeenCalledWith('MOQtailClient', 'handleRecvStreams error', expect.anything())

      errors.mockRestore()
      await client.disconnect()
    })

    it('notifies the application of the End of Range markers on a fill fetch stream', async () => {
      const { client, transport } = await connected()
      const markers: EndOfRangeKind[] = []

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        parameters: { fillParameters: { locationFilter: locationFilter(LocationFilterType.RelativeStart, 3n) } },
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
        onEndOfRange: (kind) => markers.push(kind),
      })
      const subscribeStream = await openedStream(transport, 0)
      const subscribeMsg = subscribeStream.messages[0] as Subscribe

      subscribeStream.respond(subscribeOk(7n, { largestObject: new Location(5n, 1n) }))

      const subscribed = (await subscribing) as { requestId: bigint; stream: ReadableStream<MoqtObject> }

      transport.openIncomingUniStream(
        fetchStreamBytes(subscribeMsg.requestId, [
          FetchObject.newEndOfRange(EndOfRangeKind.TimedOut, 3n, 4n),
          objectAt(4, 0),
        ]),
      )

      const reader = subscribed.stream.getReader()
      const { value } = await reader.read()

      expect(value?.location.group).toBe(4n)
      expect(markers).toEqual([EndOfRangeKind.TimedOut])
      reader.releaseLock()

      await client.disconnect()
    })

    it('delivers a fill fetch stream onto the subscription that asked for it', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        parameters: { fillParameters: { locationFilter: locationFilter(LocationFilterType.RelativeStart, 3n) } },
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      const subscribeMsg = subscribeStream.messages[0] as Subscribe

      subscribeStream.respond(subscribeOk(7n, { largestObject: new Location(5n, 1n) }))

      const subscribed = (await subscribing) as { requestId: bigint; stream: ReadableStream<MoqtObject> }

      // The fill arrives on its own uni stream, named by the SUBSCRIBE's Request ID
      // rather than a FETCH's, and carrying fetch objects.
      const header = new ByteBuffer()
      header.putBytes(Header.newFetch(FetchHeaderType.Type0x05, subscribeMsg.requestId).serialize().toUint8Array())

      const payload = new TextEncoder().encode('filled')
      header.putBytes(
        FetchObject.newObject(4, 0, 0, 0, ObjectForwardingPreference.Subgroup, null, payload)
          .serialize(undefined, GroupOrder.Ascending)
          .toUint8Array(),
      )
      transport.openIncomingUniStream(header.toUint8Array())

      const reader = subscribed.stream.getReader()
      const { value } = await reader.read()

      expect(value?.payload).toEqual(payload)
      expect(value?.location.group).toBe(4n)
      reader.releaseLock()

      // It counts towards Stream Count the same as any stream the publisher opened.
      const request = client.requests.get(subscribed.requestId) as SubscribeRequest
      expect(request.streamsAccepted).toBe(1n)

      await client.disconnect()
    })

    it('refuses a peer-opened stream that does not begin with a First-marked type', async () => {
      const { client, transport } = await connected()

      const incoming = transport.openIncomingBiStream()
      incoming.respond(requestOk())

      await vi.waitFor(() => expect(incoming.isClosed).toBe(true))
      expect(incoming.messages).toHaveLength(0)
      expect(streamResetCodeOf(incoming.abortReason)).toBe(StreamResetCode.InternalError)
      expect(streamResetCodeOf(incoming.cancelReason)).toBe(StreamResetCode.InternalError)

      await client.disconnect()
    })
  })
}
