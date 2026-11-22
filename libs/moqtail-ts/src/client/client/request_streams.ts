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

import { getHandlerForRequestStreamMessage } from '@/client/handler/handler'
import { SubscribeRequest } from '@/client/request/subscribe'
import { RequestStream } from '@/client/request_stream'
import { InternalError, ProtocolViolationError } from '@/model/error/error'
import type { GoAway, Message } from '@/model/messages'
import type { StreamResetCode } from '@/model/error/stream_reset'
import type { Tuple } from '@/model/common/tuple'
import { logger } from '@/util/logger'
import type { Session } from './session'

/**
 * Opens a bidi request stream for `first` and starts reading responses off it.
 *
 * @param requestId - The requestId carried by `first`; the key the stream is filed under.
 */
export async function openRequestStream(session: Session, requestId: bigint, first: Message): Promise<RequestStream> {
  const requestStream = await RequestStream.open(session.client.webTransport, first, session.client)
  session.requestStreams.set(requestId, requestStream)
  void pumpRequestStream(session, requestId, requestStream)

  return requestStream
}

/**
 * The stream a previously issued request runs on.
 *
 * @throws :{@link InternalError} If the request has no open stream: it was never issued,
 * or it has already completed or been cancelled.
 */
export function requestStreamFor(session: Session, requestId: bigint, context: string): RequestStream {
  const requestStream = session.requestStreams.get(requestId)
  if (!requestStream) throw new InternalError(context, `No open request stream for request id ${requestId}`)

  return requestStream
}

/** Closes the request stream filed under `requestId`, if it is still open. */
export async function closeRequestStream(session: Session, requestId: bigint): Promise<void> {
  const requestStream = session.requestStreams.get(requestId)
  if (!requestStream) return
  session.requestStreams.delete(requestId)
  await requestStream.close()
}

/** Resets the request stream filed under `requestId` with `code`, if it is still open. */
export async function resetRequestStream(session: Session, requestId: bigint, code: StreamResetCode): Promise<void> {
  const requestStream = session.requestStreams.get(requestId)
  if (!requestStream) return
  session.requestStreams.delete(requestId)
  await requestStream.reset(code)
}

/** Closes the request stream opened for `namespace`, if there is one. */
export async function closeNamespaceRequestStream(session: Session, namespace: Tuple): Promise<void> {
  const path = namespace.toUtf8Path()
  const requestId = session.namespaceRequestIds.get(path)
  if (requestId === undefined) return
  session.namespaceRequestIds.delete(path)
  await closeRequestStream(session, requestId)
}

/** The request a peer-supplied Request ID belongs to, undoing any migration. */
export function clientRequestId(session: Session, wireRequestId: bigint): bigint {
  return session.clientRequestIds.get(wireRequestId) ?? wireRequestId
}

/**
 * Re-issues one request on a fresh stream after the peer sent a GOAWAY on its request
 * stream (§10.4). Only that request moves: the session, and every other request on it,
 * are left alone.
 *
 * The re-issued request consumes a new Request ID (§10.1), so the opening message is
 * re-stamped with it. The id the caller holds keeps naming the request; only the id on
 * the wire changes.
 *
 * @returns True if the request was re-issued here. A GOAWAY naming a new session URI is
 * not: opening a session is the application's call, so it is surfaced as a `goaway`
 * event and the request stream is closed.
 */
export async function migrateRequest(session: Session, requestId: bigint, goAway: GoAway): Promise<boolean> {
  const oldStream = session.requestStreams.get(requestId)
  const first = oldStream?.first
  if (!oldStream || !first) {
    logger.warn('MOQtailClient', `GOAWAY on a request stream this side did not open (request id ${requestId})`)
    return false
  }

  if (goAway.newSessionUri) {
    session.client.emit('goaway', goAway)
    await closeRequestStream(session, requestId)
    return false
  }

  // A copy, not the original: the caller still holds that message and its id.
  const wireRequestId = session.nextRequestId()
  const reissued = { ...first, requestId: wireRequestId } as Message
  session.clientRequestIds.set(wireRequestId, requestId)

  // A subscription carries the parameters its REQUEST_UPDATEs left it with, not its first ones
  const request = session.client.requests.get(requestId)
  if (reissued.kind === 'subscribe' && request instanceof SubscribeRequest) {
    reissued.parameters = request.subscribeParameters
  }

  oldStream.migrated = true
  await oldStream.close()
  await openRequestStream(session, requestId, reissued)

  return true
}

/**
 * Reads responses and follow-ups off a locally opened request stream until the peer
 * closes it. Each message belongs to this request by virtue of the stream it arrived on,
 * so no request id is consulted to route it.
 */
export async function pumpRequestStream(
  session: Session,
  requestId: bigint,
  requestStream: RequestStream,
): Promise<void> {
  const { client } = session
  // §10.5: the response is the first message on the response stream, so anything
  // arriving here means the request was answered.
  let answered = false
  let closedBy: unknown

  try {
    while (true) {
      const msg = await requestStream.next()
      if (!msg) break

      const handler = getHandlerForRequestStreamMessage(msg)
      if (!handler) {
        throw new ProtocolViolationError(
          'MOQtailClient',
          `${msg.kind} is not valid on a request stream (request id ${requestId})`,
        )
      }

      await handler(client, msg, requestStream, requestId)
      answered = true
    }
  } catch (error) {
    if (session.closing(error)) closedBy = error
    else logger.error('MOQtailClient', `request stream for requestId=${requestId} failed`, error)

    // A subscription whose request stream breaks is over; a migrated one lives on
    const request = client.requests.get(requestId)
    if (!requestStream.migrated && request instanceof SubscribeRequest) request.finish()
  } finally {
    // A migrated request lives on under the same id on its new stream, so this one
    // ending is not the request ending.
    if (!requestStream.migrated) {
      session.requestStreams.delete(requestId)
      // A stream that dies before answering leaves the caller awaiting forever.
      if (!answered) {
        const reason =
          closedBy ?? new InternalError('MOQtailClient', `Request stream closed before answering request ${requestId}`)
        client.requests.get(requestId)?.reject(reason)
      }
    }
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { connected, openedStream, restoreWebTransport, subscribeOk } = await import('@/client/testing')
  const { FrozenByteBuffer } = await import('@/model/common/byte_buffer')
  const { Location } = await import('@/model/common/location')
  const { Tuple } = await import('@/model/common/tuple')
  const { GroupOrder, LocationFilterType } = await import('@/model/constants')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { decodeMessage, requestOk } = await import('@/model/messages')
  const { locationFilter } = await import('@/model/parameters')
  type Fetch = import('@/model/messages').Fetch
  type Subscribe = import('@/model/messages').Subscribe

  describe('MOQtailClient request streams', () => {
    restoreWebTransport()

    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('opens a stream per request and answers each on the stream it came from', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      const subscribeMsg = subscribeStream.messages[0]

      expect(subscribeMsg).toMatchObject({ kind: 'subscribe' })
      subscribeStream.respond(subscribeOk(7n, {}, []))
      expect(await subscribing).toMatchObject({ requestId: (subscribeMsg as Subscribe).requestId })

      const fetching = client.fetch({
        priority: 0,
        groupOrder: GroupOrder.Original,
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 0n, 0n, 1n),
      })
      const fetchStream = await openedStream(transport, 1)
      const fetchMsg = fetchStream.messages[0]

      expect(fetchMsg).toMatchObject({ kind: 'fetch' })
      expect((fetchMsg as Fetch).parameters.locationFilter).toEqual(
        locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 0n, 0n, 1n),
      )
      fetchStream.respond({
        kind: 'fetchOk',
        endOfTrack: false,
        endLocation: new Location(1n, 0n),
        parameters: {},
        trackProperties: [],
      })
      expect(await fetching).toMatchObject({ requestId: (fetchMsg as Fetch).requestId })

      const publishing = client.publish(ftn, true, 9n)
      const publishStream = await openedStream(transport, 2)
      const publishMsg = publishStream.messages[0]

      expect(publishMsg).toMatchObject({ kind: 'publish' })
      publishStream.respond(requestOk())
      expect(await publishing).toMatchObject({ trackAlias: 9n })

      const announcing = client.publishNamespace(Tuple.fromUtf8Path('room/alice'))
      const announceStream = await openedStream(transport, 3)
      const announceMsg = announceStream.messages[0]

      expect(announceMsg).toMatchObject({ kind: 'publishNamespace' })
      announceStream.respond(requestOk())
      expect(await announcing).toMatchObject({ kind: 'requestOk' })

      const subscribingNs = client.subscribeNamespace(Tuple.fromUtf8Path('room'))
      const subscribeNsStream = await openedStream(transport, 4)
      const subscribeNsMsg = subscribeNsStream.messages[0]

      expect(subscribeNsMsg).toMatchObject({ kind: 'subscribeNamespace' })
      subscribeNsStream.respond(requestOk())
      expect((await subscribingNs).response).toMatchObject({ kind: 'requestOk' })

      const statusing = client.trackStatus(ftn, 11n)
      const statusStream = await openedStream(transport, 5)
      const statusMsg = statusStream.messages[0]

      expect(statusMsg).toMatchObject({ kind: 'trackStatus' })
      statusStream.respond(requestOk())
      expect(await statusing).toMatchObject({ kind: 'requestOk' })

      const subscribingTracks = client.subscribeTracks(Tuple.fromUtf8Path('room'))
      const subscribeTracksStream = await openedStream(transport, 6)

      expect(subscribeTracksStream.messages[0]).toMatchObject({ kind: 'subscribeTracks' })
      subscribeTracksStream.respond(requestOk())
      expect((await subscribingTracks).response).toMatchObject({ kind: 'requestOk' })

      // All seven First-marked types, one stream each, and the control stream still
      // carries only the SETUP written during the handshake.
      expect(transport.biStreams).toHaveLength(7)
      expect(transport.sentChunks).toHaveLength(1)
      expect(decodeMessage(new FrozenByteBuffer(transport.sentChunks[0]!))).toMatchObject({ kind: 'setup' })

      await client.disconnect()
    })

    it('routes two concurrent SUBSCRIBE_OKs by stream, not by request id', async () => {
      const { client, transport } = await connected()
      const otherFtn = FullTrackName.tryNew('room/bob', 'video')

      const subscribingAlice = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const aliceStream = await openedStream(transport, 0)
      const subscribingBob = client.subscribe({
        fullTrackName: otherFtn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const bobStream = await openedStream(transport, 1)

      const aliceId = (aliceStream.messages[0] as Subscribe).requestId
      const bobId = (bobStream.messages[0] as Subscribe).requestId

      expect(aliceId).not.toBe(bobId)

      // Neither SUBSCRIBE_OK carries a request id, and they come back in the opposite
      // order to the requests: only the stream each arrives on can tell them apart.
      bobStream.respond(subscribeOk(8n, {}, []))
      aliceStream.respond(subscribeOk(7n, {}, []))

      expect(await subscribingAlice).toMatchObject({ requestId: aliceId })
      expect(await subscribingBob).toMatchObject({ requestId: bobId })

      // Each track alias resolved to the subscription that actually asked for it.
      expect(client.subscriptions.get(7n).fullTrackName.toString()).toBe(ftn.toString())
      expect(client.subscriptions.get(8n).fullTrackName.toString()).toBe(otherFtn.toString())
      expect(client.subscriptionAliasMap.get(aliceId)).toBe(7n)
      expect(client.subscriptionAliasMap.get(bobId)).toBe(8n)

      await client.disconnect()
    })

    it('closes the session on a REQUEST_UPDATE on a TRACK_STATUS stream', async () => {
      const { client, transport } = await connected()
      const terminated: unknown[] = []
      client.on('sessionterminated', (reason) => terminated.push(reason))

      const incoming = transport.openIncomingBiStream()

      incoming.respond({
        kind: 'trackStatus',
        requestId: 4n,
        trackAlias: 11n,
        fullTrackName: ftn,
        subscriberPriority: 128,
        groupOrder: GroupOrder.Original,
        forward: false,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        parameters: {},
      })
      await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))
      expect(incoming.messages[0]).toMatchObject({ kind: 'requestOk' })

      // TRACK_STATUS is not a request an update may modify, so this is a protocol
      // violation rather than something to refuse on the stream.
      incoming.respond({ kind: 'requestUpdate', requestId: 4n, parameters: { subscriberPriority: 200 } })
      await vi.waitFor(() => expect(terminated).toHaveLength(1))
      expect(incoming.messages).toHaveLength(1)
    })

    it('closes the session on a REQUEST_UPDATE on a SUBSCRIBE this side sent', async () => {
      const { client, transport } = await connected()
      const terminated: unknown[] = []
      client.on('sessionterminated', (reason) => terminated.push(reason))

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)

      subscribeStream.respond(subscribeOk(7n, {}, []))
      await subscribing

      // Only the subscriber may update a SUBSCRIBE; the publisher answering it may not.
      subscribeStream.respond({ kind: 'requestUpdate', requestId: 0n, parameters: { subscriberPriority: 200 } })
      await vi.waitFor(() => expect(terminated).toHaveLength(1))
      expect(subscribeStream.messages).toHaveLength(1)
    })

    it('re-issues the one request a GOAWAY migrates and leaves the session up', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      const requestId = (subscribeStream.messages[0] as Subscribe).requestId

      const subscribingTracks = client.subscribeTracks(Tuple.fromUtf8Path('room'))
      const tracksStream = await openedStream(transport, 1)

      // A GOAWAY on a request stream migrates that request and nothing else. No
      // URI, so it is re-issued on this same session.
      subscribeStream.respond({ kind: 'goAway', timeout: 250n })

      const reissuedStream = await openedStream(transport, 2)
      const reissued = reissuedStream.messages[0]

      expect(reissued).toMatchObject({ kind: 'subscribe' })
      expect((reissued as Subscribe).fullTrackName.toString()).toBe(ftn.toString())
      // A re-issued request consumes a fresh Request ID (§10.1)...
      expect((reissued as Subscribe).requestId).not.toBe(requestId)
      expect(subscribeStream.isClosed).toBe(true)

      // ...but the caller keeps the id it was handed, and the answer on the new stream
      // completes the call it made.
      reissuedStream.respond(subscribeOk(7n, {}, []))
      expect(await subscribing).toMatchObject({ requestId })
      expect(client.subscriptionAliasMap.get(requestId)).toBe(7n)

      // The other request rode through untouched, and the session was never torn down.
      tracksStream.respond(requestOk())
      expect((await subscribingTracks).response).toMatchObject({ kind: 'requestOk' })
      expect(transport.biStreams).toHaveLength(3)
      expect(client.goawayReceived).toBe(false)

      await client.disconnect()
    })

    it('re-issues a migrated subscription with the parameters its update left it with', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)

      subscribeStream.respond(subscribeOk(7n, {}, []))
      const { requestId } = (await subscribing) as { requestId: bigint }

      await client.subscribeUpdate({ subscriptionRequestId: requestId, forward: false, priority: 9 })
      subscribeStream.respond({ kind: 'goAway', timeout: 250n })

      const reissued = (await openedStream(transport, 1)).messages[0] as Subscribe

      expect(reissued.parameters).toMatchObject({ forward: false, subscriberPriority: 9 })

      await client.disconnect()
    })
  })
}
