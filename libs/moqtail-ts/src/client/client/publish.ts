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

import { PublishPublication } from '@/client/publication/publish'
import { PendingRequest } from '@/client/request/pending'
import type { Track } from '@/client/track/track'
import { ObjectStream } from '@/client/util/object_stream'
import { random60bitId } from '@/client/util/random_id'
import { isValidTrackAlias } from '@/client/util/validators'
import type { Tuple } from '@/model/common/tuple'
import type { FullTrackName } from '@/model/data/full_track_name'
import type { MoqtObject } from '@/model/data/object'
import { InternalError } from '@/model/error/error'
import { UNKNOWN_STREAM_COUNT, isRequestError } from '@/model/messages'
import type { Publish, PublishDone, PublishNamespace, RequestError, RequestOk } from '@/model/messages'
import type { MessageParameters } from '@/model/parameters'
import type { TrackProperty } from '@/model/property/track_property'
import { errorMessage } from '@/util/error'
import { logger } from '@/util/logger'
import { claimTrackAlias } from './aliases'
import { closeNamespaceRequestStream, closeRequestStream, openRequestStream } from './request_streams'
import { closeSession } from './session'
import type { Session } from './session'

export function addOrUpdateTrack(session: Session, track: Track): void {
  session.ensureActive()
  if (!isValidTrackAlias(track.trackAlias)) track.trackAlias = random60bitId()
  session.client.trackSources.set(track.fullTrackName.toString(), track)
}

export function removeTrack(session: Session, track: Track): void {
  session.ensureActive()
  session.client.trackSources.delete(track.fullTrackName.toString())
}

export async function publish(
  session: Session,
  fullTrackName: FullTrackName,
  forward: boolean,
  trackAlias: bigint,
  parameters: MessageParameters = {},
  trackProperties: TrackProperty[] = [],
): Promise<RequestError | { requestId: bigint; trackAlias: bigint }> {
  const { client } = session

  session.ensureActive()
  try {
    const msg: Publish = {
      kind: 'publish',
      requestId: session.nextRequestId(),
      fullTrackName,
      trackAlias,
      parameters: { ...parameters, forward },
      trackProperties,
    }
    const request = new PendingRequest(msg)

    // The alias and request id multiplex the outgoing data
    client.requests.set(msg.requestId, request)
    client.requestIdMap.addMapping(msg.requestId, fullTrackName)
    client.subscriptionAliasMap.set(msg.requestId, trackAlias)
    await openRequestStream(session, msg.requestId, msg)

    const response = await request
    if (isRequestError(response)) {
      client.requests.delete(msg.requestId)
      client.requestIdMap.removeMappingByRequestId(msg.requestId)
      client.subscriptionAliasMap.delete(msg.requestId)
      return response
    }

    return { requestId: msg.requestId, trackAlias }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.publish', errorMessage(error)))
    throw error
  }
}

export async function publishDone(
  session: Session,
  requestId: bigint,
  statusCode: number,
  reasonPhrase: string,
): Promise<void> {
  const { client } = session

  session.ensureActive()
  try {
    // It carries no request id: the stream it is sent on names the request it ends.
    // Without a publication counting its streams the exact count is unknown.
    const publication = client.publications.get(requestId)
    const streamCount = publication instanceof PublishPublication ? publication.streamsOpened : UNKNOWN_STREAM_COUNT
    const msg: PublishDone = { kind: 'publishDone', statusCode, streamCount, errorReason: reasonPhrase }

    // PUBLISH_DONE is the last message on the PUBLISH request's own stream.
    const requestStream = session.requestStreams.get(requestId)
    if (requestStream) {
      await requestStream.send(msg)
      await closeRequestStream(session, requestId)
    } else {
      logger.warn('MOQtailClient', `publishDone: no request stream for requestId=${requestId}`)
    }

    client.requests.delete(requestId)
    client.requestIdMap.removeMappingByRequestId(requestId)
    client.subscriptionAliasMap.delete(requestId)
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.publishDone', errorMessage(error)))
    throw error
  }
}

export function acceptPushedTrack(session: Session, msg: Publish): ReadableStream<MoqtObject> {
  session.ensureActive()

  // Filed under the PUBLISH's own request id, which its PUBLISH_DONE arrives under
  session.client.requestIdMap.addMapping(msg.requestId, msg.fullTrackName)

  // Stands in for a SubscribeRequest and its Subscription, so the data stream path delivers to it unchanged
  const objects = new ObjectStream()
  const receiver = {
    requestId: msg.requestId,
    fullTrackName: msg.fullTrackName,
    streamsAccepted: 0n,
    expectedStreams: undefined as bigint | undefined,
    publishDone: undefined as PublishDone | undefined,
    largestLocation: undefined,
    finish() {},
    manager: {
      objects,
      requests: [],
      drop() {},
      deliver(_request: unknown, obj: MoqtObject) {
        objects.enqueue(obj)

        return true
      },
    },
  }

  // Without a receiver for the alias, its data streams are a protocol violation
  claimTrackAlias(session, msg.trackAlias, receiver)

  return objects.readable
}

export async function publishNamespace(
  session: Session,
  trackNamespace: Tuple,
  parameters: MessageParameters = {},
): Promise<RequestOk | RequestError> {
  const { client } = session

  session.ensureActive()
  try {
    const msg: PublishNamespace = {
      kind: 'publishNamespace',
      requestId: session.nextRequestId(),
      trackNamespace,
      parameters,
    }
    const request = new PendingRequest(msg)
    client.requests.set(msg.requestId, request)
    await openRequestStream(session, msg.requestId, msg)

    const response = await request

    if (!isRequestError(response)) {
      client.publishedNamespaces.add(msg.trackNamespace)
      // The stream stays open for as long as the namespace is announced: closing it is what
      // withdraws the announcement.
      session.namespaceRequestIds.set(msg.trackNamespace.toUtf8Path(), msg.requestId)
    } else {
      await closeRequestStream(session, msg.requestId)
    }

    client.requests.delete(msg.requestId)

    return response
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.publishNamespace', errorMessage(error)))
    throw error
  }
}

export async function unpublishNamespace(session: Session, trackNamespace: Tuple): Promise<void> {
  session.ensureActive()
  try {
    session.client.publishedNamespaces.delete(trackNamespace)
    // There is no PUBLISH_NAMESPACE_DONE: closing the stream the PUBLISH_NAMESPACE opened
    // withdraws the announcement.
    await closeNamespaceRequestStream(session, trackNamespace)
  } catch (error) {
    await closeSession(session)
    throw error
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { MOQtailClient } = await import('@/client/client/client')
  const { MockWebTransport, connected, openedStream, restoreWebTransport } = await import('@/client/testing')
  const { LiveTrackSource } = await import('@/client/track/content_source')
  const { DEFAULT_MAX_FILTER_RANGES } = await import('@/client/types')
  const { ByteBuffer, FrozenByteBuffer } = await import('@/model/common/byte_buffer')
  const { Location } = await import('@/model/common/location')
  const { Tuple } = await import('@/model/common/tuple')
  const { PublishDoneStatusCode, RequestErrorCode } = await import('@/model/constants')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { Header } = await import('@/model/data/header')
  const { SubgroupObject } = await import('@/model/data/subgroup_object')
  const { decodeMessage, requestOk } = await import('@/model/messages')
  const { encodeRanges } = await import('@/model/parameters')
  type MOQtailClient = import('@/client/client/client').MOQtailClient
  type MockWebTransport = import('@/client/testing').MockWebTransport
  type Setup = import('@/model/messages').Setup

  describe('MOQtailClient publishing', () => {
    restoreWebTransport()

    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('answers a REQUEST_UPDATE on its own PUBLISH stream and keeps the publication', async () => {
      const { client, transport } = await connected()
      const moves: { requestId: bigint; forward: boolean }[] = []

      client.on('forwardstatechange', (requestId, forward) => moves.push({ requestId, forward }))

      client.addOrUpdateTrack({
        fullTrackName: ftn,
        trackSource: { live: new LiveTrackSource(new ReadableStream<MoqtObject>()) },
        publisherPriority: 0,
        trackAlias: 9n,
      })

      // FORWARD=0: nothing is sent until the peer has someone to send it to.
      const publishing = client.publish(ftn, false, 9n)
      const publishStream = await openedStream(transport, 0)

      expect(publishStream.messages[0]).toMatchObject({ kind: 'publish' })
      publishStream.respond(requestOk())

      const { requestId } = (await publishing) as { requestId: bigint }

      const publication = client.publications.get(requestId)

      expect(publication).toBeInstanceOf(PublishPublication)
      expect((publication as PublishPublication).forwarding).toBe(false)

      // A subscriber turned up, so the peer raises the Forward State on the request
      // stream the PUBLISH opened.
      publishStream.respond({ kind: 'requestUpdate', requestId: 1n, parameters: { forward: true } })

      await vi.waitFor(() => expect(publishStream.messages).toHaveLength(2))
      expect(publishStream.messages[1]).toMatchObject({ kind: 'requestOk' })
      // The update is not a reason to end the publication: it is still the one
      // registered, and it is sending now.
      expect(client.publications.get(requestId)).toBe(publication)
      expect((publication as PublishPublication).forwarding).toBe(true)
      expect(moves).toEqual([{ requestId, forward: true }])

      await client.disconnect()
    })

    it('leaves the Forward State to a REQUEST_UPDATE, not the PUBLISH_OK', async () => {
      const { client, transport } = await connected()

      client.addOrUpdateTrack({
        fullTrackName: ftn,
        trackSource: { live: new LiveTrackSource(new ReadableStream<MoqtObject>()) },
        publisherPriority: 0,
        trackAlias: 9n,
      })

      const publishing = client.publish(ftn, false, 9n)
      const publishStream = await openedStream(transport, 0)

      // A PUBLISH_OK has no subscription parameters to give; one that carries FORWARD
      // anyway does not move the publication.
      publishStream.respond(requestOk({ forward: true }))

      const { requestId } = (await publishing) as { requestId: bigint }

      expect((client.publications.get(requestId) as PublishPublication).forwarding).toBe(false)
      await client.disconnect()
    })

    it('delivers the data streams of a track the peer pushes with PUBLISH, until its PUBLISH_DONE', async () => {
      const { client, transport } = await connected()
      const errors = vi.spyOn(logger, 'error')
      const pushed = new Promise<ReadableStream<MoqtObject>>((resolve) =>
        client.on('peerpublish', (_msg, stream) => resolve(stream)),
      )

      const incoming = transport.openIncomingBiStream()

      incoming.respond({
        kind: 'publish',
        requestId: 4n,
        fullTrackName: ftn,
        trackAlias: 9n,
        parameters: {},
        trackProperties: [],
      })

      const reader = (await pushed).getReader()

      const bytes = new ByteBuffer()

      bytes.putBytes(Header.newSubgroup(0x10, 9n, 1n, undefined, 0).serialize().toUint8Array())
      bytes.putBytes(
        SubgroupObject.newWithPayload(0, null, new Uint8Array([1]))
          .serialize(undefined)
          .toUint8Array(),
      )
      transport.openIncomingUniStream(bytes.toUint8Array())

      const { value } = await reader.read()

      expect(value?.location.equals(new Location(1n, 0n))).toBe(true)
      expect(value?.fullTrackName).toEqual(ftn)
      expect(errors).not.toHaveBeenCalledWith('MOQtailClient', 'handleRecvStreams error', expect.anything())

      // Its PUBLISH_DONE accounts for the one stream, so the push ends and its alias is released
      incoming.respond({
        kind: 'publishDone',
        statusCode: PublishDoneStatusCode.TrackEnded,
        streamCount: 1n,
        errorReason: '',
      })
      expect((await reader.read()).done).toBe(true)
      expect(client.subscriptions.has(9n)).toBe(false)

      errors.mockRestore()
      await client.disconnect()
    })

    describe('received range filters', () => {
      const objectId = (setId: number, start: bigint) => ({
        setId: setId,
        deltas: encodeRanges([{ start, end: start }]),
      })

      const refused = async (client: MOQtailClient, transport: MockWebTransport, parameters: MessageParameters) => {
        const incoming = transport.openIncomingBiStream()

        incoming.respond({ kind: 'subscribe', requestId: 4n, fullTrackName: ftn, parameters })
        await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))
        expect(client.publications.has(4n)).toBe(false)

        return (incoming.messages[0] as RequestError).errorCode
      }

      it('advertises DEFAULT_MAX_FILTER_RANGES unless told otherwise', async () => {
        const { client, transport } = await connected()
        const setup = decodeMessage(new FrozenByteBuffer(transport.sentChunks[0]!)) as Setup

        expect(setup.options.maxFilterRanges).toBe(DEFAULT_MAX_FILTER_RANGES)
        expect(client.maxFilterRanges).toBe(DEFAULT_MAX_FILTER_RANGES)
        await client.disconnect()
      })

      it('refuses filters it cannot apply with INVALID_FILTER', async () => {
        const { client, transport } = await connected()

        // The same SetID twice for one filter type.
        expect(await refused(client, transport, { objectIdFilter: [objectId(0, 0n), objectId(0, 4n)] })).toBe(
          RequestErrorCode.InvalidFilter,
        )
        await client.disconnect()
      })

      it('refuses more ranges than it advertised, and any at all once it opts out', async () => {
        const one = await connected({}, { setupOptions: { maxFilterRanges: 1n } })

        expect(await refused(one.client, one.transport, { objectIdFilter: [objectId(0, 0n), objectId(1, 4n)] })).toBe(
          RequestErrorCode.InvalidFilter,
        )
        await one.client.disconnect()

        const none = await connected({}, { setupOptions: { maxFilterRanges: 0n } })

        expect(none.client.maxFilterRanges).toBe(0n)
        expect(await refused(none.client, none.transport, { objectIdFilter: [objectId(0, 0n)] })).toBe(
          RequestErrorCode.InvalidFilter,
        )
        await none.client.disconnect()
      })

      it('refuses filters on SUBSCRIBE_TRACKS as unsupported', async () => {
        const { client, transport } = await connected()
        const incoming = transport.openIncomingBiStream()

        incoming.respond({
          kind: 'subscribeTracks',
          requestId: 4n,
          trackNamespacePrefix: Tuple.fromUtf8Path('room'),
          parameters: { objectIdFilter: [objectId(0, 0n)] },
        })
        await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))
        expect((incoming.messages[0] as RequestError).errorCode).toBe(RequestErrorCode.NotSupported)
        await client.disconnect()
      })
    })
  })
}
