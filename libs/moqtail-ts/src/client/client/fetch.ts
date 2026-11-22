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

import { FetchRequest } from '@/client/request/fetch'
import { assertWithinFilterRangeLimit } from '@/client/util/validators'
import type { FetchOptions, FetchResult } from '@/client/types'
import { InternalError, ProtocolViolationError } from '@/model/error/error'
import { StreamResetCode } from '@/model/error/stream_reset'
import { isRequestError } from '@/model/messages'
import type { Fetch, RequestError } from '@/model/messages'
import { errorMessage } from '@/util/error'
import { openRequestStream, resetRequestStream } from './request_streams'
import { closeSession } from './session'
import type { Session } from './session'
import { createOrderer, requestParameters } from './subscribe'

export async function fetch(session: Session, args: FetchOptions): Promise<RequestError | FetchResult> {
  const { client } = session

  session.ensureActive()
  assertWithinFilterRangeLimit(args.parameters ?? {}, client.peerMaxFilterRanges, 'MOQtailClient.fetch')

  try {
    const { priority, groupOrder, fullTrackName, locationFilter, parameters } = args
    if (priority < 0 || priority > 255) {
      throw new ProtocolViolationError(
        'MOQtailClient.fetch',
        `subscriberPriority: ${priority} must be in range of [0-255]`,
      )
    }

    const msg: Fetch = {
      kind: 'fetch',
      requestId: session.nextRequestId(),
      fullTrackName,
      parameters: requestParameters({ priority, groupOrder, locationFilter, parameters }),
    }
    const request = new FetchRequest(msg)
    request.onEndOfRange = args.onEndOfRange
    request.orderer = createOrderer(session, args.ordering, groupOrder, msg.requestId, (obj) =>
      request.objects.enqueue(obj),
    )
    client.requests.set(msg.requestId, request)
    await openRequestStream(session, msg.requestId, msg)

    const response = await request
    if (isRequestError(response)) {
      client.requests.delete(msg.requestId)
      return response
    }

    return { requestId: msg.requestId, fetchOk: response, stream: request.stream }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.fetch', errorMessage(error)))
    throw error
  }
}

export async function fetchCancel(session: Session, requestId: bigint): Promise<void> {
  session.ensureActive()

  try {
    // There is no FETCH_CANCEL: resetting the fetch's request stream with CANCELLED is
    // the cancellation. The FetchRequest stays in `requests` so the objects already in
    // flight still resolve their track name.
    if (session.client.requests.get(requestId) instanceof FetchRequest) {
      await resetRequestStream(session, requestId, StreamResetCode.Cancelled)
    }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.fetchCancel', errorMessage(error)))
    throw error
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { connected, fetchStreamBytes, objectAt, openedStream, restoreWebTransport } = await import('@/client/testing')
  const { streamResetCodeOf } = await import('@/client/util/stream_reset')
  const { Location } = await import('@/model/common/location')
  const { GroupOrder, LocationFilterType } = await import('@/model/constants')
  const { EndOfRangeKind, FetchObject } = await import('@/model/data/fetch_object')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { locationFilter } = await import('@/model/parameters')
  const { findProperty } = await import('@/model/property/find')
  const { MaxCacheDurationProperty } = await import('@/model/property/track_property')
  const { logger } = await import('@/util/logger')
  type Location = import('@/model/common/location').Location
  type MoqtObject = import('@/model/data/object').MoqtObject
  type EndOfRangeKind = import('@/model/data/fetch_object').EndOfRangeKind

  describe('MOQtailClient fetch', () => {
    restoreWebTransport()

    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('cancels a fetch by resetting the FETCH stream with CANCELLED', async () => {
      const { client, transport } = await connected()

      const fetching = client.fetch({
        priority: 0,
        groupOrder: GroupOrder.Original,
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 0n, 0n, 1n),
      })
      const fetchStream = await openedStream(transport, 0)

      fetchStream.respond({
        kind: 'fetchOk',
        endOfTrack: false,
        endLocation: new Location(1n, 0n),
        parameters: {},
        trackProperties: [new MaxCacheDurationProperty(500n)],
      })

      const fetched = (await fetching) as FetchResult

      expect(findProperty(fetched.fetchOk.trackProperties, MaxCacheDurationProperty)?.durationMs).toBe(500n)

      await client.fetchCancel(fetched.requestId)

      expect(fetchStream.messages).toHaveLength(1)
      expect(streamResetCodeOf(fetchStream.abortReason)).toBe(StreamResetCode.Cancelled)

      await client.disconnect()
    })

    it('stops the data stream of a fetch whose consumer cancelled, without an error', async () => {
      const { client, transport } = await connected()
      const errors = vi.spyOn(logger, 'error')
      const fetching = client.fetch({
        priority: 0,
        groupOrder: GroupOrder.Original,
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.AbsoluteStart, 0n, 0n),
        ordering: { window: { durationMs: 10 } },
      })
      const fetchStream = await openedStream(transport, 0)

      fetchStream.respond({
        kind: 'fetchOk',
        endOfTrack: false,
        endLocation: new Location(9n, 0n),
        parameters: {},
        trackProperties: [],
      })

      const fetched = (await fetching) as FetchResult

      // A seek: the player cancels its reader before the FETCH is cancelled
      await fetched.stream.cancel()

      transport.openIncomingUniStream(
        fetchStreamBytes((fetchStream.messages[0] as Fetch).requestId, [objectAt(0, 0), objectAt(0, 1)]),
      )
      await vi.waitFor(() => expect(transport.uniCancelReasons).toHaveLength(1))
      expect(streamResetCodeOf(transport.uniCancelReasons[0])).toBe(StreamResetCode.Cancelled)
      expect(errors).not.toHaveBeenCalledWith('MOQtailClient', 'handleRecvStreams error', expect.anything())

      errors.mockRestore()
      await client.disconnect()
    })

    it('notifies the application of the End of Range markers in a FETCH response', async () => {
      const { client, transport } = await connected()
      const markers: [EndOfRangeKind, Location][] = []

      const fetching = client.fetch({
        priority: 0,
        groupOrder: GroupOrder.Ascending,
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 9n, 0n, 11n),
        onEndOfRange: (kind, location) => markers.push([kind, location]),
      })
      const fetchStream = await openedStream(transport, 0)
      const fetchMsg = fetchStream.messages[0] as Fetch

      fetchStream.respond({
        kind: 'fetchOk',
        endOfTrack: false,
        endLocation: new Location(11n, 0n),
        parameters: {},
        trackProperties: [],
      })

      const fetched = (await fetching) as { requestId: bigint; stream: ReadableStream<MoqtObject> }

      // The Object after the marker is encoded against it, so it arrives as {10,6}.
      transport.openIncomingUniStream(
        fetchStreamBytes(fetchMsg.requestId, [
          objectAt(9, 10),
          FetchObject.newEndOfRange(EndOfRangeKind.Unknown, 10n, 5n),
          objectAt(10, 6),
        ]),
      )

      const reader = fetched.stream.getReader()
      const first = (await reader.read()).value!
      const second = (await reader.read()).value!

      expect([first.location.group, first.location.object]).toEqual([9n, 10n])
      expect([second.location.group, second.location.object]).toEqual([10n, 6n])
      expect(markers).toHaveLength(1)
      expect(markers[0]![0]).toBe(EndOfRangeKind.Unknown)
      expect([markers[0]![1].group, markers[0]![1].object]).toEqual([10n, 5n])

      reader.releaseLock()

      await client.disconnect()
    })

    it('keeps reading a FETCH response when the End of Range handler throws', async () => {
      const { client, transport } = await connected()

      const fetching = client.fetch({
        priority: 0,
        groupOrder: GroupOrder.Ascending,
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 9n, 0n, 11n),
        onEndOfRange: () => {
          throw new Error('handler failed')
        },
      })
      const fetchStream = await openedStream(transport, 0)
      const fetchMsg = fetchStream.messages[0] as Fetch

      fetchStream.respond({
        kind: 'fetchOk',
        endOfTrack: false,
        endLocation: new Location(11n, 0n),
        parameters: {},
        trackProperties: [],
      })

      const fetched = (await fetching) as { requestId: bigint; stream: ReadableStream<MoqtObject> }

      transport.openIncomingUniStream(
        fetchStreamBytes(fetchMsg.requestId, [
          objectAt(9, 10),
          FetchObject.newEndOfRange(EndOfRangeKind.Unknown, 10n, 5n),
          objectAt(10, 6),
        ]),
      )

      const reader = fetched.stream.getReader()

      expect((await reader.read()).value!.location.object).toBe(10n)

      const afterMarker = (await reader.read()).value!

      expect([afterMarker.location.group, afterMarker.location.object]).toEqual([10n, 6n])

      reader.releaseLock()

      await client.disconnect()
    })
  })
}
