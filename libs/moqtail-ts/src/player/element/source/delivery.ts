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

import type { MOQtailClient } from '@/client/client'
import type { FetchOptions, FetchResult, SubscribeOptions, SubscribeResult, SwitchOptions } from '@/client/types'
import { Location } from '@/model/common/location'
import { GroupOrder, LocationFilterType, PublishDoneStatusCode } from '@/model/control/constant'
import { RequestError } from '@/model/control/request_error'
import { FullTrackName } from '@/model/data/full_track_name'
import type { MoqtObject } from '@/model/data/object'
import { FillParameters } from '@/model/parameter/message/fill_parameters'
import { LocationFilter } from '@/model/parameter/message/location_filter'
import type { ElementContext } from '@/player/element/context'
import type { CatalogTrack, RequestHooks, TimelineReader } from '@/player/element/interfaces'
import type { Pad, PadProperties } from '@/player/element/pad'
import { DEFAULT_MEDIA_ORDERING, MEDIA_PRIORITY } from '@/player/constants'
import { trackIdentifier } from '@/player/msf_url'
import { renditionId, trackKey } from '@/player/util/track'
import { beyond, reached, type End } from './ranges'
import { GroupTimes } from './seek'
import type { Packaging } from './switching_set'

/** The abort reason of an attempt that a new request replaces: it's ended once that one is answered. */
export const REPLACED = Symbol('replaced')

/**
 * How one request ended. `done`: it reached its end or stop point. `stopped`: destroyed, the
 * session was lost, or the source interrupted it.
 */
export type Outcome = 'ended' | 'done' | 'failed' | 'rejected' | 'stopped'

export type Accepted = SubscribeResult | FetchResult
type TrackProperties = SubscribeResult['subscribeOk']['trackProperties']

/** What a request asks for. */
export interface Target {
  /** Undefined: live from the next group, a fetch from the last delivered location + 1. */
  start?: Location
  /** The last location to deliver. Undefined: to the end of the track. */
  end?: End
  /** FETCH: VOD, or a live range behind the live edge. */
  fetch: boolean
  /** Live, `start` behind the live edge: SUBSCRIBE from the next object, filled from `start`. */
  fill?: boolean
}

/** A `switchFrom` switch in progress: objects of `track` arrive on the stream being read. */
interface SoftSwitch {
  track: CatalogTrack
  /** Where the old track ends; its later objects are dropped. */
  end: End
  /** The old track was read through `end`: its drain can stop. */
  drained?: boolean
  accepted: Promise<RequestError | SubscribeResult>
}

/** Sends a SUBSCRIBE through the hooks: they change the options last and hear the accepted result. */
async function sendSubscribe(
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

/** Sends a FETCH through the hooks, like `sendSubscribe`. */
async function sendFetch(
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

/** Sends a `SWITCH_FROM` switch; its new SUBSCRIBE goes through the hooks like `sendSubscribe`. */
export async function sendSwitch(
  client: MOQtailClient,
  hooks: RequestHooks | undefined,
  track: CatalogTrack,
  options: SwitchOptions,
): Promise<RequestError | SubscribeResult> {
  const sent = hooks?.subscribe?.(track, options.newSubscribeOptions) ?? options.newSubscribeOptions
  const result = await client.switch({ ...options, newSubscribeOptions: sent })
  if (!(result instanceof RequestError)) hooks?.subscribed?.(track, result, sent)
  return result
}

/** Writes one source's requests to its pad, in order: properties, gaps, seek answers, packets, and the `eos`. */
export class Delivery {
  /** What the pad carries now; undefined until the first `properties` item. */
  #properties: Partial<PadProperties> | undefined
  /** The track whose init data the pad carries. */
  #initFor: string | undefined
  #last: { track: string; location: Location } | undefined
  /** The request being read, and the client it was sent on. */
  #current: { client: MOQtailClient; accepted: Accepted } | undefined
  /** A subscription a new request replaces, ended once that one is answered. */
  #retiring: { client: MOQtailClient; accepted: Accepted } | undefined
  /** Where the request being read stops. */
  #stop: End | undefined
  #soft: SoftSwitch | undefined
  /** This source's accepted requests, each with the spans its `orderinggap`s skip, oldest first. */
  readonly #gaps = new Map<bigint, { from: Location; resume: Location }[]>()
  /** The client whose `orderinggap` reports are heard: the one `#gaps`' requests were sent on. */
  #listening: MOQtailClient | undefined
  /** A `track` property was written since the last packet. */
  #announced = false
  /** The publisher closed the stream being read, so there's nothing to cancel. */
  #closed = false
  /** The media timeline of the track read last, set once it's loaded. */
  #timeline: { track: string; reader?: TimelineReader | undefined } | undefined
  /** Whether the latest `play()` wrote a packet. */
  delivered = false
  /** Writes a `discontinuity` of reason `gap` before the next packet. */
  gap = false
  /** Seeks to answer with a `discontinuity` before the next packet. */
  seeks = 0
  /** `duration` for the next `properties`, in seconds, overriding VOD's `trackDuration`. */
  duration: number | undefined
  /** Called after the first packet of each track the pad announces; `changed` when it differs from the previous packet's. */
  onTrackStart: ((track: CatalogTrack, changed: boolean) => void) | undefined
  readonly times = new GroupTimes()
  readonly pad: Pad
  readonly #context: ElementContext
  readonly #hooks: RequestHooks | undefined
  readonly #packaging: Packaging

  constructor(pad: Pad, context: ElementContext, hooks: RequestHooks | undefined, packaging: Packaging) {
    this.pad = pad
    this.#context = context
    this.#hooks = hooks
    this.#packaging = packaging
    context.signal.addEventListener('abort', () => {
      this.#retire()
      this.#listening?.off('orderinggap', this.#gapped)
    })
  }

  get last(): { track: string; location: Location } | undefined {
    return this.#last
  }

  get current(): Accepted | undefined {
    return this.#current?.accepted
  }

  /**
   * Requests `track` for `target` (or reads `ready`, already sent) and reads it to the pad
   * until it ends, reaches its end, or `signal` aborts.
   */
  async play(
    track: CatalogTrack,
    signal: AbortSignal,
    target: Target,
    ready?: Promise<RequestError | Accepted>,
  ): Promise<Outcome> {
    this.delivered = false
    this.#stop = target.end
    // A live request after delivered data starts at a later group: the sink should expect the jump
    if (track.isLive && this.#last && !target.start && this.seeks === 0) this.gap = true
    const client = this.#context.client
    let accepted: RequestError | Accepted | undefined
    try {
      const request = ready ?? this.request(track, target)
      accepted = await untilAborted(request, signal, (late) => cancel(client, late))
    } catch {
      return signal.aborted ? 'stopped' : 'failed'
    } finally {
      if (!signal.aborted) this.#retire()
    }
    if (accepted === undefined) return 'stopped'
    if (accepted instanceof RequestError) return 'rejected'
    const initData = this.#initDataOf(track, trackPropertiesOf(accepted))
    if (initData === false) {
      void cancel(client, accepted)
      return 'rejected'
    }
    await this.#announce(track, initData)
    this.#current = { client, accepted }
    this.#closed = false
    const outcome = await this.#read(track, accepted, signal)
    this.#endRead(accepted, outcome, signal)
    return outcome
  }

  /** Sends the request for `target` through the hooks, and hears its `orderinggap` reports. */
  request(track: CatalogTrack, target: Target): Promise<RequestError | Accepted> {
    const client = this.#context.client
    let request: Promise<RequestError | Accepted>
    if (target.fetch) {
      const { group, object } = target.start ?? this.#resumeAt(track)
      const end = target.end
      const locationFilter = !end
        ? new LocationFilter(LocationFilterType.AbsoluteStart, group, object)
        : end.object === undefined
          ? new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, group, object, end.group)
          : new LocationFilter(LocationFilterType.AbsoluteRange, group, object, end.group, end.object)
      request = sendFetch(client, this.#hooks, track, { ...this.#common(track), locationFilter })
    } else {
      request = sendSubscribe(client, this.#hooks, track, this.subscribeOptions(track, target))
    }
    this.#hear(request)
    return request
  }

  /** Hears the `orderinggap` reports of `request` once it's accepted. */
  #hear(request: Promise<RequestError | Accepted>): void {
    const client = this.#context.client
    void request.then(
      (accepted) => {
        if (accepted instanceof RequestError) return
        if (client !== this.#listening) {
          this.#listening?.off('orderinggap', this.#gapped)
          client.on('orderinggap', this.#gapped)
          this.#listening = client
          this.#gaps.clear() // another session numbers its requests anew
        }
        this.#gaps.set(accepted.requestId, [])
      },
      () => {},
    )
  }

  /**
   * The SUBSCRIBE for a live `target`: from the next group, from `start`, or filled from `start`.
   * A filled one starts at the next object, so fill and subscription meet without a hole or an
   * overlap (moqt-22 3.4); from the next group, the rest of the current group would be lost.
   */
  subscribeOptions(track: CatalogTrack, target: Target): SubscribeOptions {
    const start = target.start
    const fill = start && target.fill
    const locationFilter = fill
      ? new LocationFilter(LocationFilterType.NextObject)
      : start
        ? new LocationFilter(LocationFilterType.AbsoluteStart, start.group, start.object)
        : new LocationFilter(LocationFilterType.RelativeStart, 0n)
    const parameters = fill
      ? [new FillParameters([new LocationFilter(LocationFilterType.AbsoluteStart, start.group, 0n)])]
      : undefined
    return { ...this.#common(track), locationFilter, ...(parameters && { parameters }) }
  }

  /** Stops the request being read after `end`; later objects aren't written. */
  stopAfter(end: End | undefined): void {
    this.#stop = end
  }

  /** Expects `track` on the stream being read, after a `SWITCH_FROM` switch; the old track's objects after `end` are dropped. */
  expectSwitch(soft: SoftSwitch | undefined): void {
    this.#soft = soft
    if (soft) this.#hear(soft.accepted)
  }

  /**
   * The old track of a `SWITCH_FROM` switch is still needed: not read through its end yet.
   * The client holds the new track's objects while the old one drains, so the drain has to
   * stop at the switch point rather than at the end of the old group.
   */
  get draining(): boolean {
    return this.#soft !== undefined && !this.#soft.drained
  }

  /** The only end signal on the pad. */
  eos(): Promise<void> {
    return this.pad.write({ type: 'event', event: { type: 'eos' } })
  }

  /** Answers every pending seek now, the latest as failed. */
  failSeeks(): Promise<void> {
    return this.#answerSeeks('failed')
  }

  /** The same track resumes after its last object; another one at the next group boundary (MSF 4.2). */
  #resumeAt(track: CatalogTrack): Location {
    const last = this.#last
    if (!last) return new Location(0n, 0n)
    if (last.track === trackKey(track)) return new Location(last.location.group, last.location.object + 1n)
    return new Location(last.location.group + 1n, 0n)
  }

  #trackProperties(track: CatalogTrack, initData: Uint8Array | undefined): Partial<PadProperties> {
    const duration =
      this.duration ?? (!track.isLive && track.trackDuration !== undefined ? track.trackDuration / 1000 : undefined)
    const latency = track.targetLatency ?? track.buffers?.target
    return {
      track: renditionId(track, this.#context.catalog),
      mimeType: this.#packaging.mimeType(track),
      live: track.isLive,
      ...(duration !== undefined && { duration }),
      ...(latency !== undefined && { targetLatency: latency / 1000 }),
      ...this.#groupProperties(track),
      ...(initData && { initData }),
    }
  }

  /** What changes as groups arrive: the observed group duration, and the media timeline's range. */
  #groupProperties(track: CatalogTrack): Partial<PadProperties> {
    const key = trackKey(track)
    const groupDuration =
      track.maxGroupDuration !== undefined ? track.maxGroupDuration / 1000 : this.times.groupDuration(key)
    const range = this.#timelineOf(track)?.range
    const current = this.#properties?.seekable
    const moved = range && (current?.start !== range.start / 1000 || current.end !== range.end / 1000)
    return {
      ...(groupDuration !== undefined && { groupDuration }),
      ...(moved && { seekable: { start: range.start / 1000, end: range.end / 1000 } }),
    }
  }

  /** The track's media timeline, once loaded: the first call starts loading it. */
  #timelineOf(track: CatalogTrack): TimelineReader | undefined {
    const key = trackKey(track)
    if (this.#timeline?.track === key) return this.#timeline.reader
    const entry: { track: string; reader?: TimelineReader | undefined } = { track: key }
    this.#timeline = entry
    void this.#context.catalog?.timeline(track).then((reader) => {
      entry.reader = reader
    })
    return undefined
  }

  async #read(track: CatalogTrack, accepted: Accepted, signal: AbortSignal): Promise<Outcome> {
    const reader = accepted.stream.getReader()
    const stop = () => void reader.cancel().catch(() => {})
    signal.addEventListener('abort', stop, { once: true })
    let endOfTrack = false
    let current = track
    // The track a `SWITCH_FROM` switch left on this stream: its late objects are dropped
    let retired: string | undefined
    try {
      for (;;) {
        const { done, value } = await reader.read()
        // An abort cancels the reader too, which also reads as done
        if (done && !signal.aborted) this.#closed = true
        if (done || signal.aborted) break
        const soft = this.#soft
        const key = soft || retired ? trackOf(value) : undefined
        if (soft && key === trackKey(soft.track)) {
          // The switch target's first object: the pad carries its track from here
          const entered = await this.#enter(soft)
          if (!entered) return 'rejected'
          retired = trackKey(current)
          current = entered
        } else if (retired && key === retired) {
          continue
        } else if (soft) {
          // The old track drains up to the switch point
          if (beyond(value.location, soft.end) || reached(value, soft.end)) soft.drained = true
          if (beyond(value.location, soft.end)) continue
        }
        // Only the current track gets here, and #enter made its request the current one
        if (!value.hasStatus() && this.#skipped(this.#current!.accepted.requestId, value.location)) continue
        const end = this.#stop
        if (end && beyond(value.location, end)) return 'done'
        if (value.isEndOfTrack()) endOfTrack = true
        // End of group carries no media, and end of track becomes the eos
        if (!value.hasStatus() && value.payload) await this.#write(current, value)
        if (end && reached(value, end)) return 'done'
      }
    } catch {
      if (!signal.aborted) return 'failed'
    } finally {
      signal.removeEventListener('abort', stop)
      reader.releaseLock()
    }
    if (signal.aborted) return 'stopped'
    const ended =
      'fetchOk' in accepted
        ? endOfTrack || accepted.fetchOk.endOfTrack
        : // MSF 11.4 says SUBSCRIBE_DONE; moqt-22 calls it PUBLISH_DONE, with TRACK_ENDED for the end
          (await accepted.done)?.statusCode === PublishDoneStatusCode.TrackEnded
    // A request with an end that closes early was cut there by the publisher, or an update
    return ended ? 'ended' : this.#stop ? 'done' : 'failed'
  }

  /**
   * After an `orderinggap` the rest of its group isn't decodable: a group starts with a SAP
   * and a chunk after a missing one can't be decoded (CMSF 3.4). Packets resume at the next
   * object 0, after a `gap` discontinuity. The report comes before the reader reaches it, so
   * objects still queued ahead of `from` are kept.
   */
  readonly #gapped = (requestId: bigint, from: Location, to: Location): void => {
    const resume = to.object === 0n ? to : new Location(to.group + 1n, 0n)
    this.#gaps.get(requestId)?.push({ from, resume })
  }

  /** Whether an object of `requestId` at `location` falls in a span an `orderinggap` skips. */
  #skipped(requestId: bigint, location: Location): boolean {
    const spans = this.#gaps.get(requestId)
    if (!spans) return false
    while (spans[0] && location.compare(spans[0].resume) >= 0) {
      spans.shift()
      this.gap = true
    }
    return spans[0] !== undefined && location.compare(spans[0].from) >= 0
  }

  /** The first object of a `SWITCH_FROM` target: its request becomes the one being read. */
  async #enter(soft: SoftSwitch): Promise<CatalogTrack | undefined> {
    this.#soft = undefined
    const accepted = await soft.accepted
    if (accepted instanceof RequestError) return undefined
    const initData = this.#initDataOf(soft.track, accepted.subscribeOk.trackProperties)
    if (initData === false) return undefined
    // Before the write: a switch issued meanwhile must see the new request as the current one
    if (this.#current) this.#current.accepted = accepted
    await this.#announce(soft.track, initData)
    return soft.track
  }

  /** Writes the properties of `track`, with the init data its request was accepted with. */
  async #announce(track: CatalogTrack, initData: Uint8Array | undefined): Promise<void> {
    await this.#setProperties(this.#trackProperties(track, initData))
    if (initData) this.#initFor = trackKey(track)
  }

  /** Forgets the request read last. A replaced subscription ends once its replacement is answered, others now. */
  #endRead(accepted: Accepted, outcome: Outcome, signal: AbortSignal): void {
    const current = this.#current!
    // `#enter` may have made a switch target's request the current one
    this.#gaps.delete(accepted.requestId)
    this.#gaps.delete(current.accepted.requestId)
    this.#current = undefined
    this.#soft = undefined
    // moqt-22 9.5 allows both subscriptions at once
    if (outcome === 'stopped' && signal.reason === REPLACED && 'subscribeOk' in current.accepted) {
      this.#retiring = current
    } else if (outcome !== 'ended' && !this.#closed) {
      void cancel(current.client, current.accepted)
    }
  }

  async #write(track: CatalogTrack, object: MoqtObject): Promise<void> {
    const key = trackKey(track)
    const initData = this.#packaging.objectInitData(track, object)
    // Packets of a track with init data on its objects can't be decoded before the first one
    if (!initData && this.#packaging.initOnObjects(track) && this.#initFor !== key) return
    if (this.seeks > 0) {
      this.gap = false
      await this.#answerSeeks('ok')
    } else if (this.gap) {
      this.gap = false
      await this.pad.write({ type: 'event', event: { type: 'discontinuity', reason: 'gap' } })
    }
    if (initData) {
      await this.#setProperties({ initData })
      this.#initFor = key
    }
    await this.pad.write({ type: 'packet', object })
    const previous = this.#last
    this.#last = { track: key, location: object.location }
    this.delivered = true
    if (previous?.location.group !== object.location.group || previous.track !== key) {
      this.#recordTime(track, object)
      await this.#setProperties(this.#groupProperties(track))
    }
    const changed = previous !== undefined && previous.track !== key
    // Also after a `track` property, as when a failed switch target falls back to the old track
    if (changed || this.#announced) this.onTrackStart?.(track, changed)
    this.#announced = false
  }

  /** A `discontinuity` for each pending seek, the latest as failed for `failed`. */
  async #answerSeeks(outcome: 'ok' | 'failed'): Promise<void> {
    while (this.seeks > 0) {
      this.seeks--
      const failed = outcome === 'failed' && this.seeks === 0
      await this.pad.write({
        type: 'event',
        event: { type: 'discontinuity', reason: 'seek', ...(failed && { failed }) },
      })
    }
  }

  /** The media time of a group's first delivered object. */
  #recordTime(track: CatalogTrack, object: MoqtObject): void {
    const time = this.#packaging.mediaTime(track, object, this.#properties?.initData)
    if (time !== undefined) this.times.record(trackKey(track), object.location.group, time)
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
    if (changed['track'] !== undefined) this.#announced = true
    await this.pad.write({ type: 'properties', properties: changed })
  }

  /** What every request of `track` carries; the hooks may change it. */
  #common(track: CatalogTrack) {
    return {
      fullTrackName: FullTrackName.tryNew(track.namespace, track.nameBytes),
      priority: MEDIA_PRIORITY,
      groupOrder: GroupOrder.Original,
      ordering: this.#context.player.options.objectOrdering ?? DEFAULT_MEDIA_ORDERING,
    }
  }

  /** The init data a request of `track` was accepted with, or `false` when it's invalid. */
  #initDataOf(track: CatalogTrack, trackProperties: TrackProperties): Uint8Array | undefined | false {
    try {
      return this.#packaging.acceptedInitData(track, trackProperties)
    } catch {
      return false
    }
  }

  #retire(): void {
    const retiring = this.#retiring
    this.#retiring = undefined
    if (retiring) void cancel(retiring.client, retiring.accepted)
  }
}

function trackPropertiesOf(accepted: Accepted): TrackProperties {
  return 'fetchOk' in accepted ? accepted.fetchOk.trackProperties : accepted.subscribeOk.trackProperties
}

/** The key of the track an object or a request is for. */
export function trackOf(named: { fullTrackName: FullTrackName }): string {
  return trackIdentifier(named.fullTrackName.namespace, named.fullTrackName.name)
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
export async function cancel(client: MOQtailClient, result: RequestError | Accepted): Promise<void> {
  if (result instanceof RequestError) return
  try {
    await ('fetchOk' in result ? client.fetchCancel(result.requestId) : client.unsubscribe(result.requestId))
  } catch {
    // The client is gone; so is the request
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack, endOfTrack, fakeCatalog, timedObject, twoSecondTimeline } =
    await import('@/player/testing/cmaf')
  const { mediaObject } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')

  describe('Delivery', () => {
    test('live: SUBSCRIBE from the next group at priority 128, properties first, eos once on TRACK_ENDED', async () => {
      const c = await cmafChain([cmafTrack('v1', { init: { type: 'inline', data: new Uint8Array([9]) } })])
      await flush()
      const [sent] = c.client.sent<SubscribeOptions>('subscribe')
      expect(sent).toMatchObject({ priority: 128, groupOrder: GroupOrder.Original })
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

    test('live timing: target latency, observed group duration, and the timeline range as seconds', async () => {
      const timeline = { ...twoSecondTimeline(), range: { start: 0, end: 4000 } }
      const c = await cmafChain([cmafTrack('v1', { buffers: { target: 1500 }, timescale: 1000 })], {
        catalog: fakeCatalog([], timeline),
      })
      await flush()
      c.client.last.push(timedObject(1, 0), timedObject(2, 2))
      await flush()
      timeline.range = { start: 2000, end: 6000 }
      c.client.last.push(timedObject(3, 4))
      await flush()
      expect(c.sink.items().flatMap((i) => (i.type === 'properties' ? [i.properties] : []))).toEqual([
        expect.objectContaining({ targetLatency: 1.5 }),
        { seekable: { start: 0, end: 4 } },
        { groupDuration: 2 },
        { seekable: { start: 2, end: 6 } },
      ])
    })

    test("the sink's backpressure stops the read loop", async () => {
      const blocked = Promise.withResolvers<void>()
      const c = await cmafChain([cmafTrack('v1')])
      c.sink.slow = () => blocked.promise
      await flush()
      for (let i = 0; i < 50; i++) c.client.last.push(mediaObject(1, i))
      await flush()
      // A connection queues 32 items, so most of the 50 objects stay unread
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

    test('orderinggap: the rest of the group is dropped, late objects too, then a gap before the next group', async () => {
      const c = await cmafChain([cmafTrack('v1')])
      await flush()
      expect(c.client.sent<SubscribeOptions>('subscribe')[0]!.ordering).toEqual({ window: { durationMs: 150 } })
      c.client.last.push(mediaObject(1, 0))
      await flush()
      c.client.emit('orderinggap', 0n, new Location(1n, 1n), new Location(1n, 3n))
      c.client.emit('orderinggap', 9n, new Location(2n, 0n), new Location(2n, 1n)) // another request's
      c.client.last.push(mediaObject(1, 3), mediaObject(1, 1), mediaObject(2, 0), mediaObject(2, 1))
      await flush()
      // Whole groups missing: packets resume at the next one
      c.client.emit('orderinggap', 0n, new Location(3n, 0n), new Location(4n, 0n))
      c.client.last.push(mediaObject(4, 0))
      await flush()
      expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', 'gap', '2.0', '2.1', 'gap', '4.0'])
    })

    test('orderinggap: objects released before the gap but still unread are kept', async () => {
      const blocked = Promise.withResolvers<void>()
      const c = await cmafChain([cmafTrack('v1')])
      c.sink.slow = () => blocked.promise
      await flush()
      for (let i = 0; i < 50; i++) c.client.last.push(mediaObject(1, i))
      c.client.last.push(mediaObject(2, 0))
      await flush()
      c.client.emit('orderinggap', 0n, new Location(2n, 1n), new Location(2n, 2n))
      c.client.last.push(mediaObject(2, 2), mediaObject(3, 0))
      blocked.resolve()
      await flush()
      expect(c.trace().slice(-5)).toEqual(['1.48', '1.49', '2.0', 'gap', '3.0'])
    })

    test('a catalog namespace other than the track’s prefixes the track id', async () => {
      const catalog = Object.assign(fakeCatalog(), { namespace: new Tuple([TupleField.fromUtf8('other')]) })
      const c = await cmafChain([cmafTrack('v1')], { catalog })
      await flush()
      expect(c.sink.items()[0]).toMatchObject({ properties: { track: 'ns--v1' } })
    })
  })

  const { Tuple, TupleField } = await import('@/model/common/tuple')
}
