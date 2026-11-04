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

/**
 * The path from a client request to the source pad: the request helpers every SUBSCRIBE
 * and FETCH goes through, the `properties` the pad carries, and the read loop.
 */

import type { MOQtailClient } from '../../../client/client'
import type { FetchOptions, FetchResult, SubscribeOptions, SubscribeResult } from '../../../client/types'
import type { Location } from '../../../model/common/location'
import { GroupOrder, LocationFilterType, PublishDoneStatusCode } from '../../../model/control/constant'
import { RequestError } from '../../../model/control/request_error'
import { FullTrackName } from '../../../model/data/full_track_name'
import type { MoqtObject } from '../../../model/data/object'
import { LocationFilter } from '../../../model/parameter/message/location_filter'
import type { ElementContext } from '../../element/context'
import type { CatalogTrack, RequestHooks } from '../../element/interfaces'
import type { Pad, PadProperties } from '../../element/pad'
import { acceptedInitData, objectInitData } from './init_data'
import { mimeTypeOf, renditionId, trackKey } from './track'

/** Media ranks below the catalog's 0 (MSF 5); 128 is the moqt-22 default. */
export const MEDIA_PRIORITY = 128

/** How one request ended. `stopped`: destroyed, the session was lost, or the source interrupted it. */
export type Outcome = 'ended' | 'failed' | 'rejected' | 'stopped'

type Accepted = SubscribeResult | FetchResult

/** Sends a SUBSCRIBE through the hooks: they change the options last and hear the accepted result. */
export async function subscribe(
  client: MOQtailClient,
  hooks: RequestHooks | undefined,
  track: CatalogTrack,
  options: SubscribeOptions,
): Promise<RequestError | SubscribeResult> {
  const sent = hooks?.subscribe?.(track, options) ?? options
  const result = await client.subscribe(sent)
  if (!(result instanceof RequestError)) hooks?.subscribed?.(track, result, sent)
  return result
}

/** Sends a FETCH through the hooks, like `subscribe`. */
export async function fetch(
  client: MOQtailClient,
  hooks: RequestHooks | undefined,
  track: CatalogTrack,
  options: FetchOptions,
): Promise<RequestError | FetchResult> {
  const sent = hooks?.fetch?.(track, options) ?? options
  const result = await client.fetch(sent)
  if (!(result instanceof RequestError)) hooks?.fetched?.(track, result, sent)
  return result
}

/** Writes one source's requests to its pad, in order: properties, gaps, packets, and the `eos`. */
export class Delivery {
  /** What the pad carries now; undefined until the first `properties` item. */
  #properties: Partial<PadProperties> | undefined
  /** The track whose init data the pad carries. */
  #initFor: string | undefined
  #last: { track: string; location: Location } | undefined
  #gap = false
  /** Whether the latest `play()` wrote a packet. */
  delivered = false

  constructor(
    readonly pad: Pad,
    readonly context: ElementContext,
    readonly hooks: RequestHooks | undefined,
  ) {}

  /**
   * Requests `track` from where delivery stands, live from the next group and VOD from the
   * last delivered location + 1, and reads it to the pad until it ends or `signal` aborts.
   */
  async play(track: CatalogTrack, signal: AbortSignal): Promise<Outcome> {
    this.delivered = false
    // A live request after delivered data starts at a later group: the sink should expect the jump.
    this.#gap = track.isLive && this.#last !== undefined
    const client = this.context.client
    let accepted: RequestError | Accepted | undefined
    try {
      accepted = await untilAborted(this.#request(client, track), signal, (late) => cancel(client, late))
    } catch {
      return signal.aborted ? 'stopped' : 'failed'
    }
    if (accepted === undefined) return 'stopped'
    if (accepted instanceof RequestError) return 'rejected'
    let initData: Uint8Array | undefined
    try {
      initData = acceptedInitData(
        track,
        'fetchOk' in accepted ? accepted.fetchOk.trackProperties : accepted.subscribeOk.trackProperties,
      )
    } catch {
      void cancel(client, accepted)
      return 'rejected'
    }
    await this.#setProperties({
      track: renditionId(track, this.context.catalog),
      mimeType: mimeTypeOf(track),
      live: track.isLive,
      ...(track.trackDuration !== undefined && !track.isLive && { duration: track.trackDuration / 1000 }),
      ...(initData && { initData }),
    })
    if (initData) this.#initFor = trackKey(track)
    const outcome = await this.#read(track, accepted, signal)
    if (outcome !== 'ended') void cancel(client, accepted)
    return outcome
  }

  /** The only end signal on the pad. */
  eos(): Promise<void> {
    return this.pad.write({ type: 'event', event: { type: 'eos' } })
  }

  #request(client: MOQtailClient, track: CatalogTrack): Promise<RequestError | Accepted> {
    const fullTrackName = FullTrackName.tryNew(track.namespace, track.nameBytes)
    const common = { fullTrackName, priority: MEDIA_PRIORITY, groupOrder: GroupOrder.Original }
    if (track.isLive) {
      const locationFilter = new LocationFilter(LocationFilterType.RelativeStart, 0n)
      return subscribe(client, this.hooks, track, { ...common, forward: true, locationFilter })
    }
    const last = this.#last
    // The same track resumes after its last object; another one at the next group boundary (MSF 4.2).
    const [group, object] = !last
      ? [0n, 0n]
      : last.track === trackKey(track)
        ? [last.location.group, last.location.object + 1n]
        : [last.location.group + 1n, 0n]
    const locationFilter = new LocationFilter(LocationFilterType.AbsoluteStart, group, object)
    return fetch(client, this.hooks, track, { ...common, locationFilter })
  }

  async #read(track: CatalogTrack, accepted: Accepted, signal: AbortSignal): Promise<Outcome> {
    const reader = accepted.stream.getReader()
    const stop = () => void reader.cancel().catch(() => {})
    signal.addEventListener('abort', stop, { once: true })
    let endOfTrack = false
    try {
      for (;;) {
        const { done, value } = await reader.read()
        if (done || signal.aborted) break
        if (value.isEndOfTrack()) endOfTrack = true
        // End of group carries no media, and end of track becomes the eos.
        if (!value.hasStatus() && value.payload) await this.#write(track, value)
      }
    } catch {
      if (!signal.aborted) return 'failed'
    } finally {
      signal.removeEventListener('abort', stop)
      reader.releaseLock()
    }
    if (signal.aborted) return 'stopped'
    if ('fetchOk' in accepted) return endOfTrack || accepted.fetchOk.endOfTrack ? 'ended' : 'failed'
    // MSF 11.4 says SUBSCRIBE_DONE; moqt-22 calls it PUBLISH_DONE, with TRACK_ENDED for the end.
    return (await accepted.done)?.statusCode === PublishDoneStatusCode.TrackEnded ? 'ended' : 'failed'
  }

  async #write(track: CatalogTrack, object: MoqtObject): Promise<void> {
    const key = trackKey(track)
    const initData = objectInitData(track, object.properties)
    // Packets of an object-property track can't be decoded before its first init data.
    if (!initData && track.init?.type === 'object-property' && this.#initFor !== key) return
    if (this.#gap) {
      this.#gap = false
      await this.pad.write({ type: 'event', event: { type: 'discontinuity', reason: 'gap' } })
    }
    if (initData) {
      await this.#setProperties({ initData })
      this.#initFor = key
    }
    await this.pad.write({ type: 'packet', object })
    this.#last = { track: key, location: object.location }
    this.delivered = true
  }

  /** Writes what changed; the first item carries everything. */
  async #setProperties(next: Partial<PadProperties>): Promise<void> {
    const changed: Record<string, unknown> = {}
    for (const [key, value] of Object.entries(next)) {
      const current = (this.#properties as Record<string, unknown> | undefined)?.[key]
      if (value !== undefined && !sameValue(current, value)) changed[key] = value
    }
    if (this.#properties && Object.keys(changed).length === 0) return
    this.#properties = { ...this.#properties, ...changed }
    await this.pad.write({ type: 'properties', properties: changed })
  }
}

function sameValue(a: unknown, b: unknown): boolean {
  if (a instanceof Uint8Array && b instanceof Uint8Array) return a.length === b.length && a.every((x, i) => x === b[i])
  return a === b
}

/** The request's result, or undefined once `signal` aborts first; a late result goes to `late`. */
function untilAborted<T>(request: Promise<T>, signal: AbortSignal, late: (result: T) => void): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    const onAbort = () => {
      request.then(late, () => {})
      resolve(undefined)
    }
    if (signal.aborted) return onAbort()
    signal.addEventListener('abort', onAbort, { once: true })
    request.then(resolve, reject).finally(() => signal.removeEventListener('abort', onAbort))
  })
}

/** Ends a request this source no longer reads. A lost session already ended it. */
async function cancel(client: MOQtailClient, result: RequestError | Accepted): Promise<void> {
  if (result instanceof RequestError) return
  try {
    await ('fetchOk' in result ? client.fetchCancel(result.requestId) : client.unsubscribe(result.requestId))
  } catch {
    // The client is gone; so is the request.
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack, endOfTrack, fakeCatalog, requested } = await import('../../testing/cmaf')
  const { mediaObject } = await import('../../testing/client')
  const { flush } = await import('../../testing/elements')
  const { UnknownTrackProperty } = await import('../../../model/property/track_property')
  const { VideoConfig } = await import('../../../model/property/video_config')
  const init = (byte: number) => new VideoConfig(new Uint8Array([byte]))

  describe('Delivery', () => {
    test('live: SUBSCRIBE from the next group at priority 128, properties first, eos once on TRACK_ENDED', async () => {
      const c = await cmafChain([cmafTrack('v1', { init: { type: 'inline', data: new Uint8Array([9]) } })])
      await flush()
      const [sent] = c.client.sent<SubscribeOptions>('subscribe')
      expect(sent).toMatchObject({ priority: 128, groupOrder: GroupOrder.Original, forward: true })
      expect(sent!.locationFilter).toMatchObject({ filterType: LocationFilterType.RelativeStart, startGroup: 0n })

      c.client.last.push(mediaObject(1, 0), mediaObject(1, 1), endOfTrack(1, 2))
      c.client.last.end(PublishDoneStatusCode.TrackEnded)
      await flush()
      expect(c.trace()).toEqual(['props initData,live,mimeType,track', '1.0', '1.1', 'eos'])
      expect(c.sink.items()[0]).toMatchObject({
        properties: { track: 'v1', mimeType: 'video/mp4; codecs="avc1.64001f"', live: true },
      })
    })

    test('VOD: FETCH of the whole track, with its duration, ending after its end of track', async () => {
      const c = await cmafChain([cmafTrack('v1', { isLive: false, trackDuration: 4000 })], {
        respond: () => ({ endOfTrack: true }),
      })
      await flush()
      const [sent] = c.client.sent<FetchOptions>('fetch')
      expect(sent).toMatchObject({ priority: 128 })
      expect(sent!.locationFilter).toMatchObject({
        filterType: LocationFilterType.AbsoluteStart,
        startGroup: 0n,
        startObject: 0n,
      })
      c.client.last.push(mediaObject(1))
      c.client.last.end()
      await flush()
      expect(c.trace()).toEqual(['props duration,live,mimeType,track', '1.0', 'eos'])
      expect(c.sink.items()[0]).toMatchObject({ properties: { live: false, duration: 4 } })
    })

    test("the sink's backpressure stops the read loop", async () => {
      const blocked = Promise.withResolvers<void>()
      const c = await cmafChain([cmafTrack('v1')])
      c.sink.slow = () => blocked.promise
      await flush()
      for (let i = 0; i < 50; i++) c.client.last.push(mediaObject(1, i))
      await flush()
      // A connection queues 32 items, so most of the 50 objects stay unread.
      expect(c.client.last.unread).toBeGreaterThan(10)
      blocked.resolve()
      await flush()
      expect(c.client.last.unread).toBe(0)
    })

    test('request hooks change what is sent, and hear the accepted result', async () => {
      const heard: unknown[] = []
      const requests: RequestHooks = {
        subscribe: (_, options) => ({ ...options, priority: 7 }),
        subscribed: (track, result, options) => heard.push([track.name, result.requestId, options.priority]),
      }
      const c = await cmafChain([cmafTrack('v1')], { cmaf: { requests } })
      await flush()
      expect(c.client.sent<SubscribeOptions>('subscribe')[0]!.priority).toBe(7)
      expect(heard).toEqual([['v1', 0n, 7]])
    })

    test('init data from a track property, and a promised one that is missing fails the rendition', async () => {
      const property = { type: 'track-property' as const, property: VideoConfig.TYPE }
      const trackProperties = [new UnknownTrackProperty(init(5).toKeyValuePair())]
      const c = await cmafChain([cmafTrack('v1', { init: property }), cmafTrack('v2', { init: property })], {
        respond: (_, options) => (requested(options) === 'v2' ? { trackProperties } : {}),
      })
      await flush()
      // v1 had nothing, so the set moved on to v2 and unsubscribed v1.
      expect(c.client.sent('unsubscribe')).toEqual([0n])
      expect(c.sink.items()).toMatchObject([{ properties: { track: 'v2', initData: new Uint8Array([5]) } }])
    })

    test('object-property init data: packets before it are dropped, a change writes properties again', async () => {
      const c = await cmafChain([cmafTrack('v1', { init: { type: 'object-property', property: VideoConfig.TYPE } })])
      await flush()
      const [a, b] = [init(1).toKeyValuePair(), init(2).toKeyValuePair()]
      c.client.last.push(mediaObject(1, 0), mediaObject(1, 1, [a]), mediaObject(1, 2, [a]), mediaObject(2, 0, [b]))
      await flush()
      expect(c.trace()).toEqual(['props live,mimeType,track', 'props initData', '1.1', '1.2', 'props initData', '2.0'])
      expect(c.sink.items()[4]).toMatchObject({ properties: { initData: new Uint8Array([2]) } })
    })

    test('a catalog namespace other than the track’s prefixes the track id', async () => {
      const catalog = Object.assign(fakeCatalog(), { namespace: new Tuple([TupleField.fromUtf8('other')]) })
      const c = await cmafChain([cmafTrack('v1')], { catalog })
      await flush()
      expect(c.sink.items()[0]).toMatchObject({ properties: { track: 'ns--v1' } })
    })
  })

  const { Tuple, TupleField } = await import('../../../model/common/tuple')
}
