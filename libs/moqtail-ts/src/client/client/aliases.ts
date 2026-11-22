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

import type { TrackAliasHolder } from '@/client/types'
import type { Session } from './session'

/**
 * Records the alias a publisher just named for `holder`, releasing any data stream that
 * arrived before the control message carrying it.
 */
export function claimTrackAlias(session: Session, trackAlias: bigint, holder: TrackAliasHolder): void {
  const { client } = session

  client.subscriptions.set(trackAlias, holder)
  client.subscriptionAliasMap.set(holder.requestId, trackAlias)
  client.aliasFullTrackNameMap.set(trackAlias, holder.fullTrackName)

  const waiters = session.aliasWaiters.get(trackAlias)
  if (!waiters) return

  session.aliasWaiters.delete(trackAlias)
  for (const waiter of waiters) waiter()
}

/**
 * Waits out the race between a subscription's data streams and the control message that
 * names their track alias. The two travel on separate streams, so nothing orders them
 * and the data can arrive first; giving up on it immediately would drop a whole
 * subgroup, a visible gap in the media.
 *
 * Returns once the alias is claimed, or once `trackAliasResolutionTimeoutMs` elapses with
 * it still unclaimed.
 */
export function waitForTrackAlias(session: Session, trackAlias: bigint): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>()
  let waiters = session.aliasWaiters.get(trackAlias)

  if (!waiters) {
    waiters = new Set()
    session.aliasWaiters.set(trackAlias, waiters)
  }

  const claimed = () => {
    clearTimeout(timeoutId)
    resolve()
  }

  const timeoutId = setTimeout(() => {
    waiters.delete(claimed)
    if (waiters.size === 0) session.aliasWaiters.delete(trackAlias)

    resolve()
  }, session.client.trackAliasResolutionTimeoutMs)

  waiters.add(claimed)

  return promise
}

/**
 * Drops `holder`'s hold on its track alias.
 *
 * An alias names a track, not a subscription, so two requests for the same track carry
 * the same one: a switch back to a track whose earlier request has not been retired yet
 * is exactly that. The alias-keyed maps can only describe the request that holds the
 * alias now, so the other one clears its own row and leaves theirs alone.
 */
export function releaseTrackAlias(session: Session, holder: TrackAliasHolder): void {
  const { client } = session
  const trackAlias = client.subscriptionAliasMap.get(holder.requestId)

  client.subscriptionAliasMap.delete(holder.requestId)

  if (trackAlias === undefined) return
  if (client.subscriptions.get(trackAlias) !== holder) return

  client.subscriptions.delete(trackAlias)
  client.aliasFullTrackNameMap.delete(trackAlias)
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  const { connected, openedStream, restoreWebTransport, subscribeOk } = await import('@/client/testing')
  const { ByteBuffer } = await import('@/model/common/byte_buffer')
  const { GroupOrder, LocationFilterType } = await import('@/model/constants')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { Header } = await import('@/model/data/header')
  const { SubgroupObject } = await import('@/model/data/subgroup_object')
  const { locationFilter } = await import('@/model/parameters')
  type DataHeaderEvent = import('@/client/data_event').DataHeaderEvent
  type MoqtObject = import('@/model/data/object').MoqtObject

  describe('MOQtailClient track aliases', () => {
    restoreWebTransport()
    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('holds a data stream that outran the control message naming its track alias', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      let header: DataHeaderEvent | undefined

      client.on('dataevent', (event) => {
        if (event.kind === 'header') header = event
      })

      // The subgroup and the SUBSCRIBE_OK travel on separate streams, so the data can
      // land first. Dropping it would cost the whole subgroup.
      const bytes = new ByteBuffer()
      bytes.putBytes(Header.newSubgroup(0x10, 7n, 4n, undefined, 0).serialize().toUint8Array())

      const payload = new TextEncoder().encode('early')
      bytes.putBytes(SubgroupObject.newWithPayload(0, null, payload).serialize(undefined).toUint8Array())

      transport.openIncomingUniStream(bytes.toUint8Array())

      subscribeStream.respond(subscribeOk(7n, {}, []))

      const subscribed = (await subscribing) as { requestId: bigint; stream: ReadableStream<MoqtObject> }
      const reader = subscribed.stream.getReader()
      const { value } = await reader.read()

      expect(value?.payload).toEqual(payload)
      expect(value?.location.group).toBe(4n)
      expect(value?.fullTrackName.toString()).toBe(ftn.toString())
      // Unnamed when the stream opened, named once the alias is
      expect(header?.fullTrackName).toBeUndefined()
      expect(client.trackNameOf(header!)?.toString()).toBe(ftn.toString())

      reader.releaseLock()

      await client.disconnect()
    })

    it('abandons a data stream whose track alias is never named', async () => {
      const { client, transport } = await connected()

      client.trackAliasResolutionTimeoutMs = 10

      const bytes = new ByteBuffer()
      bytes.putBytes(Header.newSubgroup(0x10, 9n, 4n, undefined, 0).serialize().toUint8Array())
      bytes.putBytes(
        SubgroupObject.newWithPayload(0, null, new TextEncoder().encode('orphan')).serialize(undefined).toUint8Array(),
      )

      transport.openIncomingUniStream(bytes.toUint8Array())

      await new Promise((resolve) => setTimeout(resolve, 50))

      // The wait is over well before the alias is named, so the stream is gone and
      // the subscription that eventually claims 9 never sees what it carried.
      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })

      ;(await openedStream(transport, 0)).respond(subscribeOk(9n, {}, []))

      const subscribed = (await subscribing) as { requestId: bigint; stream: ReadableStream<MoqtObject> }
      const reader = subscribed.stream.getReader()
      const delivered = await Promise.race([
        reader.read().then(() => 'delivered'),
        new Promise((resolve) => setTimeout(() => resolve('nothing'), 50)),
      ])
      expect(delivered).toBe('nothing')

      reader.releaseLock()

      await client.disconnect()
    })
  })
}
