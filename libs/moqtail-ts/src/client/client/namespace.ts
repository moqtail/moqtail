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

import { PendingRequest } from '@/client/request/pending'
import { RequestStream } from '@/client/request_stream'
import { assertWithinFilterRangeLimit } from '@/client/util/validators'
import type { Tuple } from '@/model/common/tuple'
import { GroupOrder, LocationFilterType } from '@/model/constants'
import type { FullTrackName } from '@/model/data/full_track_name'
import { InternalError, ProtocolViolationError } from '@/model/error/error'
import { isRequestError } from '@/model/messages'
import type { RequestError, RequestOk, SubscribeNamespace, SubscribeTracks, TrackStatus } from '@/model/messages'
import { locationFilter } from '@/model/parameters'
import type { MessageParameters } from '@/model/parameters'
import { errorMessage } from '@/util/error'
import {
  closeNamespaceRequestStream,
  closeRequestStream,
  openRequestStream,
  pumpRequestStream,
} from './request_streams'
import { closeSession } from './session'
import type { Session } from './session'

/** What a SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS answers with, and how to end it. */
interface PrefixSubscription {
  response: RequestOk | RequestError
  cancel: () => Promise<void>
}

export async function subscribeNamespace(
  session: Session,
  prefix: Tuple,
  parameters: MessageParameters = {},
): Promise<PrefixSubscription> {
  const { client } = session

  session.ensureActive()

  try {
    const msg: SubscribeNamespace = {
      kind: 'subscribeNamespace',
      requestId: session.nextRequestId(),
      trackNamespacePrefix: prefix,
      parameters,
    }

    // NAMESPACE and NAMESPACE_DONE carry only a suffix, so they mean something only next to
    // the prefix that opened this stream. The generic pump has no prefix to hand them,
    // hence the stream is read by drainNamespaceStream instead.
    const requestStream = await RequestStream.open(client.webTransport, msg, client)
    session.requestStreams.set(msg.requestId, requestStream)

    const response = await firstResponse(requestStream, 'MOQtailClient.subscribeNamespace')
    if (!isRequestError(response)) {
      client.subscribedNamespaces.add(prefix)
      session.namespaceRequestIds.set(prefix.toUtf8Path(), msg.requestId)

      void drainNamespaceStream(session, requestStream, msg.requestId, prefix)
    } else {
      await closeRequestStream(session, msg.requestId)
    }

    const cancel = async () => {
      client.subscribedNamespaces.delete(prefix)
      session.namespaceRequestIds.delete(prefix.toUtf8Path())
      await closeRequestStream(session, msg.requestId)
    }

    return { response, cancel }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.subscribeNamespace', errorMessage(error)))
    throw error
  }
}

export async function subscribeTracks(
  session: Session,
  prefix: Tuple,
  parameters: MessageParameters = {},
): Promise<PrefixSubscription> {
  const { client } = session

  session.ensureActive()
  assertWithinFilterRangeLimit(parameters, client.peerMaxFilterRanges, 'MOQtailClient.subscribeTracks')

  try {
    const msg: SubscribeTracks = {
      kind: 'subscribeTracks',
      requestId: session.nextRequestId(),
      trackNamespacePrefix: prefix,
      parameters,
    }
    const requestStream = await RequestStream.open(client.webTransport, msg, client)
    session.requestStreams.set(msg.requestId, requestStream)

    const response = await firstResponse(requestStream, 'MOQtailClient.subscribeTracks')
    if (!isRequestError(response)) {
      client.subscribedTracks.add(prefix)

      void pumpRequestStream(session, msg.requestId, requestStream)
    } else {
      await closeRequestStream(session, msg.requestId)
    }

    const cancel = async () => {
      client.subscribedTracks.delete(prefix)
      await closeRequestStream(session, msg.requestId)
    }

    return { response, cancel }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.subscribeTracks', errorMessage(error)))
    throw error
  }
}

export async function unsubscribeNamespace(session: Session, prefix: Tuple): Promise<void> {
  session.ensureActive()

  try {
    // There is no UNSUBSCRIBE_NAMESPACE: closing the stream the SUBSCRIBE_NAMESPACE opened
    // ends the prefix subscription.
    session.client.subscribedNamespaces.delete(prefix)

    await closeNamespaceRequestStream(session, prefix)
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.unsubscribeNamespace', errorMessage(error)))
    throw error
  }
}

export async function trackStatus(
  session: Session,
  fullTrackName: FullTrackName,
  trackAlias: bigint,
  parameters: MessageParameters = {},
): Promise<RequestOk | RequestError> {
  const { client } = session

  session.ensureActive()

  try {
    const msg: TrackStatus = {
      kind: 'trackStatus',
      requestId: session.nextRequestId(),
      trackAlias,
      fullTrackName,
      subscriberPriority: 128,
      groupOrder: GroupOrder.Original,
      forward: false,
      locationFilter: locationFilter(LocationFilterType.NextObject),
      parameters,
    }
    const request = new PendingRequest(msg)
    client.requests.set(msg.requestId, request)
    await openRequestStream(session, msg.requestId, msg)

    const response = await request

    await closeRequestStream(session, msg.requestId)
    client.requests.delete(msg.requestId)

    return response
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.trackStatus', errorMessage(error)))
    throw error
  }
}

/** The REQUEST_OK or REQUEST_ERROR that opens the response side of a prefix subscription. */
async function firstResponse(requestStream: RequestStream, context: string): Promise<RequestOk | RequestError> {
  const response = await requestStream.next()
  if (!response) throw new InternalError(context, 'Stream closed before response')
  if (response.kind !== 'requestOk' && response.kind !== 'requestError') {
    throw new ProtocolViolationError(context, 'Unexpected response message type')
  }

  return response
}

async function drainNamespaceStream(
  session: Session,
  requestStream: RequestStream,
  requestId: bigint,
  prefix: Tuple,
): Promise<void> {
  try {
    while (true) {
      const msg = await requestStream.next()
      if (!msg) break

      if (msg.kind === 'namespace') session.client.emit('peernamespace', prefix, msg.trackNamespaceSuffix)
      else if (msg.kind === 'namespaceDone') session.client.emit('peernamespacedone', prefix, msg.trackNamespaceSuffix)
    }
  } finally {
    session.requestStreams.delete(requestId)
    session.namespaceRequestIds.delete(prefix.toUtf8Path())
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { connected, openedStream, restoreWebTransport } = await import('@/client/testing')
  const { Tuple } = await import('@/model/common/tuple')
  const { RequestErrorCode } = await import('@/model/constants')
  const { requestError, requestOk } = await import('@/model/messages')
  const { ObjectDeliveryTimeoutProperty } = await import('@/model/property/track_property')
  type PublishSkipped = import('@/model/messages').PublishSkipped

  describe('MOQtailClient namespaces', () => {
    restoreWebTransport()

    it('holds one prefix under both SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS', async () => {
      const { client, transport } = await connected()
      const prefix = Tuple.fromUtf8Path('room')

      const subscribingNs = client.subscribeNamespace(prefix)
      const nsStream = await openedStream(transport, 0)

      expect(nsStream.messages[0]).toMatchObject({ kind: 'subscribeNamespace' })

      nsStream.respond(requestOk())

      expect((await subscribingNs).response).toMatchObject({ kind: 'requestOk' })

      const subscribingTracks = client.subscribeTracks(prefix)
      const tracksStream = await openedStream(transport, 1)

      expect(tracksStream.messages[0]).toMatchObject({ kind: 'subscribeTracks' })

      tracksStream.respond(requestOk())

      expect((await subscribingTracks).response).toMatchObject({ kind: 'requestOk' })

      // Independent overlap spaces (§10.19), so the prefix is tracked twice over.
      expect(client.subscribedNamespaces.has(prefix)).toBe(true)
      expect(client.subscribedTracks.has(prefix)).toBe(true)

      await client.disconnect()
    })

    it('surfaces PUBLISH_SKIPPED on the SUBSCRIBE_TRACKS response stream', async () => {
      const { client, transport } = await connected()
      const prefix = Tuple.fromUtf8Path('room')
      const blocked: { prefix: Tuple; msg: PublishSkipped }[] = []

      client.on('peerpublishskipped', (subscribedPrefix, msg) => {
        blocked.push({ prefix: subscribedPrefix, msg })
      })

      const subscribingTracks = client.subscribeTracks(prefix)
      const tracksStream = await openedStream(transport, 0)

      tracksStream.respond(requestOk())

      expect((await subscribingTracks).response).toMatchObject({ kind: 'requestOk' })

      tracksStream.respond({
        kind: 'publishSkipped',
        trackNamespaceSuffix: Tuple.fromUtf8Path('alice'),
        trackName: new TextEncoder().encode('video'),
      })

      await vi.waitFor(() => expect(blocked).toHaveLength(1))
      // The message carries the suffix only; the prefix comes from the stream it
      // arrived on, so the blocked track is room/alice:video.
      expect(blocked[0]!.prefix.equals(prefix)).toBe(true)
      expect(blocked[0]!.msg.trackNamespaceSuffix.toUtf8Path()).toBe('/alice')
      expect(new TextDecoder().decode(blocked[0]!.msg.trackName)).toBe('video')

      await client.disconnect()
    })

    it('surfaces PREFIX_OVERLAP for a second overlapping SUBSCRIBE_TRACKS', async () => {
      const { client, transport } = await connected()

      const first = client.subscribeTracks(Tuple.fromUtf8Path('room'))
      const firstStream = await openedStream(transport, 0)

      firstStream.respond(requestOk())

      expect((await first).response).toMatchObject({ kind: 'requestOk' })

      const second = client.subscribeTracks(Tuple.fromUtf8Path('room/alice'))
      const secondStream = await openedStream(transport, 1)

      expect(secondStream.messages[0]).toMatchObject({ kind: 'subscribeTracks' })

      secondStream.respond(requestError(RequestErrorCode.PrefixOverlap, 'overlaps', 0n))

      const { response } = await second

      expect(response).toMatchObject({ kind: 'requestError' })
      expect((response as RequestError).errorCode).toBe(RequestErrorCode.PrefixOverlap)
      expect(client.subscribedTracks.size).toBe(1)

      await client.disconnect()
    })

    it('refuses a REQUEST_OK carrying Track Properties outside a TRACK_STATUS_OK', async () => {
      const { client, transport } = await connected()

      const announcing = client.publishNamespace(Tuple.fromUtf8Path('room/alice'))
      const announceStream = await openedStream(transport, 0)

      // §10.5 populates Track Properties in a TRACK_STATUS_OK and nowhere else, so this
      // PUBLISH_NAMESPACE_OK is a protocol violation: the request fails rather than
      // resolving with a namespace the peer never really accepted.
      announceStream.respond(requestOk({}, [new ObjectDeliveryTimeoutProperty(5000n)]))

      // Without the properties the same exchange resolves; see the request-per-stream
      // test above.
      await expect(announcing).rejects.toThrow()
    })

    it('closes the bidi stream when a namespace request update fails', async () => {
      const { client, transport } = await connected()

      const incoming = transport.openIncomingBiStream()
      incoming.respond({
        kind: 'subscribeNamespace',
        requestId: 4n,
        trackNamespacePrefix: Tuple.fromUtf8Path('room'),
        parameters: {},
      })

      await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))

      expect(incoming.messages[0]).toMatchObject({ kind: 'requestOk' })

      // The client keeps no prefix-subscription state to update, so §10.9.1 ends the
      // request by closing its stream.
      incoming.respond({ kind: 'requestUpdate', requestId: 4n, parameters: {} })

      await vi.waitFor(() => expect(incoming.messages).toHaveLength(2))

      expect(incoming.messages[1]).toMatchObject({ kind: 'requestError' })

      await vi.waitFor(() => expect(incoming.isClosed).toBe(true))

      await client.disconnect()
    })
  })
}
