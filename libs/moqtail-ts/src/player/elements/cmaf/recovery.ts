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
 * What keeps a CMAF source playing (`PLAN.md` › Lifecycle And Errors): retries with backoff,
 * falling back to another rendition, resuming after a reconnect, and holding an `eos` until
 * the catalog confirms the end (MSF 11.4).
 */

import type { ElementContext } from '../../element/context'
import type { CatalogTrack } from '../../element/interfaces'
import { backoffDelay, sleep } from '../../util/backoff'
import type { Delivery } from './delivery'
import { bitrateOf, trackKey } from './track'

/** What recovery needs from its source. */
export interface RecoveryHost {
  /** The selected tracks of the set. */
  candidates(): readonly CatalogTrack[]
  /** No rendition is left: the source removes its pad. */
  giveUp(): void
}

export class Recovery {
  /** The track being requested. */
  active: CatalogTrack
  /** Renditions that were rejected, or failed too often. */
  readonly #failed = new Set<string>()
  /** Re-subscribes so far while an `eos` is held; undefined when not holding. */
  #holding: number | undefined
  /** Aborted by a lost session, or to release a held `eos`. */
  #attempt = new AbortController()
  #reconnect: PromiseWithResolvers<void> | undefined

  constructor(
    readonly context: ElementContext,
    readonly delivery: Delivery,
    readonly host: RecoveryHost,
    start: CatalogTrack,
  ) {
    this.active = start
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
    })
  }

  /** Plays until the `eos`, until no rendition is left, or until the source is destroyed. */
  async run(): Promise<void> {
    const { signal } = this.context
    const maxFailures = this.context.player.options.client?.reconnectAttempts ?? 5
    let failures = 0
    while (!signal.aborted) {
      await this.#reconnect?.promise // a lost session isn't a failure, it waits for the new one
      if (signal.aborted) return
      if (this.#holding !== undefined && this.#isFinal()) return this.delivery.eos()
      this.#attempt = new AbortController()
      const attempt = AbortSignal.any([this.#attempt.signal, signal])
      // While holding, the publisher may restart: re-subscribe with backoff, capped at 5 s.
      if (this.#holding !== undefined && !(await pause(backoffDelay(this.#holding++), attempt))) continue

      const outcome = await this.delivery.play(this.active, attempt)
      if (this.delivery.delivered) {
        failures = 0
        this.#holding = undefined
      }
      if (outcome === 'stopped') continue
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
      if (!this.#fallback()) return
    }
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
