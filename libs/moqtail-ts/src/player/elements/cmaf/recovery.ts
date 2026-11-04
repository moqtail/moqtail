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
 * What keeps a CMAF source playing (`PLAN.md` › Lifecycle And Errors, Runtime Control): retries
 * with backoff, falling back to another rendition, resuming after a reconnect, holding an `eos`
 * until the catalog confirms the end (MSF 11.4), seeks, switch handovers, and URL ranges.
 */

import type { MOQtailClient } from '../../../client/client'
import type { Location } from '../../../model/common/location'
import type { RequestError } from '../../../model/control/request_error'
import type { ElementContext } from '../../element/context'
import type { CatalogTrack } from '../../element/interfaces'
import { backoffDelay, sleep } from '../../util/backoff'
import { cancel, REPLACED, type Accepted, type Delivery, type Target } from './delivery'
import { clamp, mapEnd, mapLocation, type End, type Span } from './ranges'
import { locate, seekTarget } from './seek'
import { bitrateOf, trackKey } from './track'

/** What recovery needs from its source. */
export interface RecoveryHost {
  /** The selected tracks of the set. */
  candidates(): readonly CatalogTrack[]
  /** No rendition is left: the source removes its pad. */
  giveUp(): void
}

/** A switch's request, sent while the old track plays to its switch point. */
interface Handover {
  track: CatalogTrack
  request: Promise<RequestError | Accepted>
  client: MOQtailClient
  start: Location
  /** `SWITCH_FROM`: the new track arrives on the stream being read. */
  soft: boolean
}

export class Recovery {
  /** The track being requested. */
  active: CatalogTrack
  /** What the next request asks for. */
  #target: Target
  /** The set's first Group ID (MSF 6.3.2). */
  readonly g0: bigint
  /** Renditions that were rejected, or failed too often. */
  readonly #failed = new Set<string>()
  /** Re-subscribes so far while an `eos` is held; undefined when not holding. */
  #holding: number | undefined
  /** Aborted by a lost session, or to release a held `eos`. */
  #attempt = new AbortController()
  #reconnect: PromiseWithResolvers<void> | undefined
  /** Set while nothing is selected; resolved by a new selection. */
  #paused: PromiseWithResolvers<void> | undefined
  /** The latest seek not requested yet, in seconds. */
  #seekTo: number | undefined
  #handover: Handover | undefined
  /** URL ranges, against the set's first track (`#first`). */
  readonly #spans: readonly Span[]
  #span = 0
  readonly #first: CatalogTrack

  constructor(
    readonly context: ElementContext,
    readonly delivery: Delivery,
    readonly host: RecoveryHost,
    start: CatalogTrack,
    ranges: { spans: readonly Span[]; g0: bigint } = { spans: [], g0: 1n },
  ) {
    this.active = this.#first = start
    this.#spans = ranges.spans
    this.g0 = ranges.g0
    this.#target = this.#spans[0] ? this.#spanTarget(this.#spans[0]) : { fetch: !start.isLive }
    const { player, catalog, signal } = context
    const lost = () => {
      this.#reconnect ??= Promise.withResolvers()
      this.#attempt.abort()
    }
    const back = () => {
      this.#reconnect?.resolve()
      this.#reconnect = undefined
    }
    const changed = () => {
      if (this.#holding !== undefined && this.#isFinal()) this.#attempt.abort()
    }
    player.on('reconnecting', lost)
    player.on('reconnected', back)
    catalog?.on('change', changed)
    signal.addEventListener('abort', () => {
      player.off('reconnecting', lost)
      player.off('reconnected', back)
      catalog?.off('change', changed)
      back()
      this.resume()
    })
  }

  /** Plays until the `eos`, until no rendition is left, or until the source is destroyed. */
  async run(): Promise<void> {
    const { signal } = this.context
    const maxFailures = this.context.player.options.client?.reconnectAttempts ?? 5
    let failures = 0
    let ready: Promise<RequestError | Accepted> | undefined
    while (!signal.aborted) {
      await this.#reconnect?.promise // a lost session isn't a failure, it waits for the new one
      await this.#paused?.promise
      if (signal.aborted) return
      if (this.#holding !== undefined && this.#isFinal()) return this.delivery.eos()
      this.#attempt = new AbortController()
      const attempt = AbortSignal.any([this.#attempt.signal, signal])
      if (this.#seekTo !== undefined) await this.#locateSeek()
      if (attempt.aborted) continue
      // While holding, the publisher may restart: re-subscribe with backoff, capped at 5 s.
      if (this.#holding !== undefined && !(await pause(backoffDelay(this.#holding++), attempt))) continue

      const outcome = await this.delivery.play(this.active, attempt, { ...this.#target, ...this.#end() }, ready)
      ready = undefined
      if (this.delivery.delivered) {
        failures = 0
        this.#holding = undefined
        this.#target = { fetch: this.#target.fetch } // a retry resumes from where delivery stands
      }
      const handover = this.#handover
      if (handover) {
        // The old track reached its switch point, or stopped early: continue on the new one.
        this.#handover = undefined
        this.active = handover.track
        this.#target = { fetch: this.#target.fetch, start: handover.start }
        if (outcome !== 'stopped' && !handover.soft) ready = handover.request
        else void handover.request.then((late) => cancel(handover.client, late))
        continue
      }
      if (outcome === 'stopped') continue
      if (outcome === 'done') {
        if (this.#spans.length === 0) continue
        if (this.#nextSpan()) continue
        return this.delivery.eos()
      }
      if (outcome === 'ended') {
        if (this.#isFinal()) return this.delivery.eos()
        this.#holding ??= 0
        continue
      }
      if (this.#holding !== undefined) continue // while holding, a rejection is just another try
      if (outcome === 'failed') {
        if (!(await pause(backoffDelay(failures), attempt))) continue
        if (++failures < maxFailures) continue
      }
      failures = 0
      await this.delivery.failSeeks() // a seek that couldn't be served is answered as failed
      if (!this.#fallback()) return
    }
  }

  /** Requests the group containing `mediaTime` (seconds); answered with one `discontinuity`. */
  seek(mediaTime: number): void {
    this.delivery.seeks++
    this.#seekTo = mediaTime
    // A switch during a seek resumes on the new track.
    this.#takeHandover()
    this.#attempt.abort(REPLACED)
  }

  /** A seek is pending: not requested, or not answered yet. */
  get seeking(): boolean {
    return this.#seekTo !== undefined || this.delivery.seeks > 0
  }

  /** Requests `track` from where delivery stands; nothing was delivered to hand over from. */
  restartOn(track: CatalogTrack): void {
    this.active = track
    this.#attempt.abort(REPLACED)
  }

  /** Continues on `track` with `request` once the track being read reaches its stop point. */
  handOver(track: CatalogTrack, request: Promise<RequestError | Accepted>, start: Location, soft: boolean): void {
    this.#handover = { track, request, client: this.context.client, start, soft }
  }

  /** The track a pending handover continues on. */
  get handingOverTo(): CatalogTrack | undefined {
    return this.#handover?.track
  }

  /** Forgets the handover to `track`: it started, or its request was rejected. */
  dropHandover(track: CatalogTrack): void {
    if (this.#handover?.track === track) this.#handover = undefined
  }

  /** Stops requesting, without counting it as a failure, until `resume()`. */
  pause(): void {
    this.#paused ??= Promise.withResolvers()
    this.#takeHandover()
    this.#attempt.abort()
  }

  resume(): void {
    this.#paused?.resolve()
    this.#paused = undefined
  }

  /** The end of the range being played, for `track`. */
  rangeEnd(track: CatalogTrack): End | undefined {
    const end = this.#spans[this.#span]?.end
    return end && mapEnd(end, this.#first, track, this.g0)
  }

  /** The range being played, if any, as an `end` for `active`. */
  #end(): { end?: End } {
    const end = this.rangeEnd(this.active)
    return end ? { end } : {}
  }

  #takeHandover(): void {
    const handover = this.#handover
    if (!handover) return
    this.#handover = undefined
    this.active = handover.track
    void handover.request.then((late) => cancel(handover.client, late))
  }

  #spanTarget(span: Span): Target {
    const start = mapLocation(span.start, this.#first, this.active, this.g0)
    return span.fetch ? { fetch: true, start } : { fetch: false, start, fill: true }
  }

  /** Moves to the next range, after a gap. */
  #nextSpan(): boolean {
    const next = this.#spans[++this.#span]
    if (!next) return false
    this.#target = this.#spanTarget(next)
    this.delivery.gap = true
    return true
  }

  /** Turns the latest seek into `target`, or answers it as failed when it can't be located. */
  async #locateSeek(): Promise<void> {
    const time = this.#seekTo!
    this.#seekTo = undefined
    const track = this.active
    const timeline = await this.context.catalog?.timeline(track)
    if (this.#seekTo !== undefined || track !== this.active) return // superseded meanwhile
    const key = trackKey(track)
    let location = locate(time, track, timeline, (t) => this.delivery.times.estimate(key, t))
    if (!location) return this.delivery.failSeeks()
    let fetch = !track.isLive
    if (this.#spans.length > 0) {
      const clamped = clamp(mapLocation(location, track, this.#first, this.g0), this.#spans)
      this.#span = clamped.index
      location = mapLocation(clamped.location, this.#first, track, this.g0)
      fetch = this.#spans[clamped.index]!.fetch
    }
    const last = this.delivery.last
    this.#target = seekTarget(location, fetch, last?.track === key ? last.location : undefined)
  }

  /** Moves to the selected rendition closest in bitrate, or gives up. */
  #fallback(): boolean {
    this.#failed.add(trackKey(this.active))
    const next = closestBitrate(
      this.host.candidates().filter((t) => !this.#failed.has(trackKey(t))),
      bitrateOf(this.active),
    )
    if (!next) {
      this.host.giveUp()
      return false
    }
    this.active = next
    return true
  }

  /** The end is final once the track isn't live anymore or the catalog is complete. */
  #isFinal(): boolean {
    const catalog = this.context.catalog
    if (!catalog || catalog.isComplete) return true
    const key = trackKey(this.active)
    return !(catalog.tracks.find((t) => trackKey(t) === key) ?? this.active).isLive
  }
}

export function closestBitrate(tracks: readonly CatalogTrack[], bitrate: number): CatalogTrack | undefined {
  let best: CatalogTrack | undefined
  for (const track of tracks) {
    if (!best || Math.abs(bitrateOf(track) - bitrate) < Math.abs(bitrateOf(best) - bitrate)) best = track
  }
  return best
}

/** Waits `ms`; false if `signal` aborted first. */
const pause = (ms: number, signal: AbortSignal): Promise<boolean> =>
  sleep(ms, signal).then(
    () => true,
    () => false,
  )

if (import.meta.vitest) {
  const { describe, test, expect, vi, afterEach } = import.meta.vitest
  const { cmafChain, cmafTrack, fakeCatalog, requested } = await import('../../testing/cmaf')
  const { mediaObject, refuse } = await import('../../testing/client')
  const { flush } = await import('../../testing/elements')
  const { PublishDoneStatusCode } = await import('../../../model/control/constant')
  type Sent = { fullTrackName: import('../../../model/data/full_track_name').FullTrackName; locationFilter?: any }
  /** Runs every timer and what it starts; the longest backoff is 5 s. */
  const elapse = () => vi.advanceTimersByTimeAsync(6000)
  afterEach(() => void vi.useRealTimers())

  describe('Recovery', () => {
    test('a rejected rendition falls back to the closest bitrate, and none left removes the pad', async () => {
      const set = [
        cmafTrack('v1', { bitrate: 500 }),
        cmafTrack('v2', { bitrate: 3000 }),
        cmafTrack('v3', { bitrate: 1000 }),
      ]
      const c = await cmafChain(set, { respond: (_, o) => (requested(o) === 'v3' ? {} : refuse()) })
      await flush()
      expect(c.client.sent<Sent>('subscribe').map(requested)).toEqual(['v1', 'v3'])
      expect(c.source.getRenditions().find((r) => r.active)?.id).toBe('v3')

      const lonely = await cmafChain([cmafTrack('v1')], { respond: () => refuse() })
      await flush()
      expect(lonely.source.pads).toEqual([])
    })

    test('a reset retries with backoff: live from the next group after a gap, then falls back', async () => {
      vi.useFakeTimers()
      const heard: string[] = []
      const requests = { subscribed: (track: CatalogTrack) => void heard.push(track.name) }
      const c = await cmafChain([cmafTrack('v1'), cmafTrack('v2')], { cmaf: { requests }, reconnectAttempts: 2 })
      await elapse()
      c.client.last.push(mediaObject(1))
      await elapse()
      c.client.last.reset()
      await elapse()
      c.client.last.push(mediaObject(3))
      c.client.last.end(PublishDoneStatusCode.Expired) // not TRACK_ENDED: a failure too
      await elapse()
      c.client.last.reset()
      await elapse()
      // Two failures in a row without data move on to v2.
      expect(heard).toEqual(['v1', 'v1', 'v1', 'v2'])
      expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', 'gap', '3.0', 'props track'])
    })

    test('VOD retries from the last delivered location + 1', async () => {
      vi.useFakeTimers()
      const c = await cmafChain([cmafTrack('v1', { isLive: false })])
      await elapse()
      c.client.last.push(mediaObject(2, 4))
      await elapse()
      c.client.last.reset()
      await elapse()
      const filter = c.client.sent<Sent>('fetch')[1]!.locationFilter
      expect([filter.startGroup, filter.startObject]).toEqual([2n, 5n])
      expect(c.trace()).not.toContain('gap')
    })

    test('TRACK_ENDED before the catalog says so holds the eos, re-subscribes, and resumes after a gap', async () => {
      vi.useFakeTimers()
      const catalog = fakeCatalog()
      let subscribes = 0
      const c = await cmafChain([cmafTrack('v1')], { catalog, respond: () => (++subscribes > 1 ? refuse() : {}) })
      await elapse()
      c.client.last.push(mediaObject(1))
      c.client.last.end(PublishDoneStatusCode.TrackEnded)
      await elapse()
      await elapse()
      // Rejections while holding aren't failures: the source keeps trying and keeps its pad.
      expect(c.client.sent('subscribe').length).toBeGreaterThan(2)
      expect(c.source.pads).toHaveLength(1)
      c.client.respond = () => ({})
      await elapse()
      c.client.last.push(mediaObject(5))
      await elapse()
      expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', 'gap', '5.0'])

      c.client.last.end(PublishDoneStatusCode.TrackEnded)
      await elapse()
      catalog.isComplete = true
      catalog.emit('change')
      await elapse()
      expect(c.trace().slice(-1)).toEqual(['eos'])
      const sent = c.client.sent('subscribe').length
      await elapse()
      expect(c.client.sent('subscribe').length).toBe(sent)
    })

    test('a lost session is not a failure: live resumes on the new client after a gap', async () => {
      vi.useFakeTimers()
      const c = await cmafChain([cmafTrack('v1'), cmafTrack('v2')], { reconnectAttempts: 1 })
      await elapse()
      c.client.last.push(mediaObject(1))
      await elapse()
      c.lose()
      await elapse()
      c.reconnected()
      await elapse()
      expect(c.client.sent<Sent>('subscribe').map(requested)).toEqual(['v1'])
      c.client.last.push(mediaObject(7))
      await elapse()
      expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', 'gap', '7.0'])
    })

    test('a lost session resumes VOD from the last delivered location + 1', async () => {
      vi.useFakeTimers()
      const c = await cmafChain([cmafTrack('v1', { isLive: false })])
      await elapse()
      c.client.last.push(mediaObject(3, 1))
      await elapse()
      c.lose()
      c.reconnected()
      await elapse()
      const filter = c.client.sent<Sent>('fetch')[0]!.locationFilter
      expect([filter.startGroup, filter.startObject]).toEqual([3n, 2n])
    })

    test('destroy unsubscribes, including a subscribe still pending', async () => {
      const pending = Promise.withResolvers<{}>()
      const c = await cmafChain([cmafTrack('v1')], { respond: () => pending.promise })
      await flush()
      await c.chain.destroy()
      pending.resolve({})
      await flush()
      expect(c.client.sent('unsubscribe')).toEqual([0n])

      const streaming = await cmafChain([cmafTrack('v1')])
      await flush()
      await streaming.chain.destroy()
      await flush()
      expect(streaming.client.sent('unsubscribe')).toEqual([0n])
    })
  })
}
