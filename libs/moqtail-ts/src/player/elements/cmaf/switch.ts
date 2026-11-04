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
 * Rendition switching (`PLAN.md` › Runtime Control): the switch point (MSF 6.3.2), and the
 * default, `SWITCH_FROM`, and VOD request sequences, which share the switch point, the
 * `properties` written for the new track, and dropping the old track past its end.
 */

import type { SubscribeResult, SwitchOptions } from '../../../client/types'
import { Location } from '../../../model/common/location'
import { LocationFilterType, SwitchMode } from '../../../model/control/constant'
import { RequestError } from '../../../model/control/request_error'
import { FillParameters } from '../../../model/parameter/message/fill_parameters'
import { LocationFilter } from '../../../model/parameter/message/location_filter'
import type { ElementContext } from '../../element/context'
import type { CatalogTrack, MediaSourceOptions } from '../../element/interfaces'
import { PlayerError } from '../../error'
import { switchTo, trackOf, type Delivery } from './delivery'
import type { End } from './ranges'
import type { Recovery } from './recovery'
import { trackKey } from './track'

/**
 * Where a switch from `from` to `to` happens, after `last` (the last delivered location of
 * `from`): the target's first group, and the old track's last location.
 * With `keyframeSpacing` on every track of the set (MSF 6.3.2, counted from `g0`), the target
 * group is the first `k` with `k × T` past the delivered frames; otherwise the next group.
 */
export function switchPoint(
  from: CatalogTrack,
  to: CatalogTrack,
  last: Location,
  set: readonly CatalogTrack[],
  g0: bigint,
): { group: bigint; end: End } {
  if (!from.keyframeSpacing || !to.keyframeSpacing || !set.every((t) => t.keyframeSpacing)) {
    return { group: last.group + 1n, end: { group: last.group } }
  }
  const [s, t] = [BigInt(from.keyframeSpacing), BigInt(to.keyframeSpacing)]
  const next = (last.group - g0) * s + last.object + 1n // the first frame not delivered yet
  const k = (next + t - 1n) / t
  const endFrame = k * t - 1n
  return { group: g0 + k, end: { group: g0 + endFrame / s, object: endFrame % s } }
}

/** What a switch needs from its source. */
export interface SwitchHost {
  readonly context: ElementContext
  readonly options: MediaSourceOptions
  readonly delivery: Delivery
  readonly recovery: Recovery
  /** The switching set. */
  tracks(): readonly CatalogTrack[]
}

interface Waiter {
  track: string
  from: string
  resolve: () => void
  reject: (error: unknown) => void
}

/** Runs one switch at a time; with concurrent calls the latest wins and earlier ones reject. */
export class Switcher {
  #latest = 0
  #lock = Promise.resolve()
  readonly #waiters = new Set<Waiter>()

  constructor(readonly host: SwitchHost) {
    host.context.signal.addEventListener('abort', () => {
      for (const waiter of this.#waiters) waiter.reject(aborted('The source was destroyed'))
    })
  }

  /** Resolves after the `properties` item and the first packet of `track` are written. */
  async switch(track: CatalogTrack): Promise<void> {
    const token = ++this.#latest
    const previous = this.#lock
    const done = Promise.withResolvers<void>()
    this.#lock = done.promise
    try {
      await previous
      if (token !== this.#latest) throw aborted('A later switch superseded this one')
      const from = this.host.recovery.active
      if (trackKey(track) === trackKey(from)) return
      const started = this.#started(track, from)
      try {
        await this.#start(track)
      } catch (error) {
        started.cancel()
        throw error
      }
      await started.promise
    } finally {
      done.resolve()
    }
  }

  /** Called by the source after the first packet of `track` was written. */
  started(track: CatalogTrack): void {
    const key = trackKey(track)
    const handingOverTo = this.host.recovery.handingOverTo
    for (const waiter of this.#waiters) {
      if (waiter.track === key) waiter.resolve()
      // The old track may play on only until the handover; anything else means the target failed.
      else if (waiter.from !== key || (handingOverTo && trackKey(handingOverTo)) !== waiter.track)
        waiter.reject(new PlayerError('UNSUPPORTED_SOURCE', 'The switch target failed', { element: 'cmaf' }))
    }
  }

  #started(track: CatalogTrack, from: CatalogTrack) {
    const { promise, resolve, reject } = Promise.withResolvers<void>()
    const waiter: Waiter = {
      track: trackKey(track),
      from: trackKey(from),
      resolve: () => (this.#waiters.delete(waiter), resolve()),
      reject: (error) => (this.#waiters.delete(waiter), reject(error)),
    }
    this.#waiters.add(waiter)
    return { promise, cancel: () => this.#waiters.delete(waiter) }
  }

  async #start(track: CatalogTrack): Promise<void> {
    const { recovery, delivery, context } = this.host
    const from = recovery.active
    const last = delivery.last
    const current = delivery.current
    // Nothing of the old track to hand over from, or a seek resumes on the new track anyway.
    if (recovery.seeking || !current || last?.track !== trackKey(from)) return recovery.restartOn(track)

    const point = switchPoint(from, track, last.location, this.host.tracks(), recovery.g0)
    const start = new Location(point.group, 0n)
    if (this.host.options.switchFrom && 'subscribeOk' in current) return this.#soft(track, from, current, point)

    const fetch = 'fetchOk' in current
    const end = recovery.rangeEnd(track)
    delivery.stopAfter(point.end)
    const request = delivery.request(track, { fetch, start, ...(fetch && end && { end }) })
    recovery.handOver(track, request, start, false)
    const accepted = await request
    if (accepted instanceof RequestError) return this.#rejected(track, from)
    if (fetch) return
    // The old subscription ends at the switch point: its last group, or its last object for a fast switch.
    const locationFilter =
      point.end.object === undefined
        ? new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, last.location.group, 0n, point.end.group)
        : new LocationFilter(
            LocationFilterType.AbsoluteRange,
            last.location.group,
            0n,
            point.end.group,
            point.end.object,
          )
    // Without the update, reading still stops at the switch point.
    await context.client.subscribeUpdate({ subscriptionRequestId: current.requestId, locationFilter }).catch(() => {})
  }

  /** `SWITCH_FROM` in soft mode: objects of both tracks arrive on the stream being read. */
  async #soft(track: CatalogTrack, from: CatalogTrack, current: SubscribeResult, point: { group: bigint; end: End }) {
    const { recovery, delivery, context, options } = this.host
    const start = new Location(point.group, 0n)
    const fromKey = trackKey(from)
    const newSubscribeOptions = {
      ...delivery.subscribeOptions(track, { fetch: false, start }),
      // Already-published objects from the switch group on.
      parameters: [new FillParameters([])],
    }
    const request = switchTo(context.client, options.requests, track, {
      switchFromRequestId: current.requestId,
      switchMode: SwitchMode.Soft,
      newSubscribeOptions,
      // As demo-abr: only the track switched from drains, and only until the new one arrives.
      onDrainDecision: (request) => delivery.switching && trackOf(request) === fromKey,
    })
    delivery.expectSwitch({ track, end: point.end, accepted: request })
    recovery.handOver(track, request, start, true)
    if ((await request) instanceof RequestError) {
      delivery.expectSwitch(undefined)
      return this.#rejected(track, from)
    }
  }

  #rejected(track: CatalogTrack, from: CatalogTrack): never {
    this.host.recovery.dropHandover(track)
    this.host.delivery.stopAfter(this.host.recovery.rangeEnd(from))
    throw new PlayerError('UNSUPPORTED_SOURCE', `The switch to ${track.name} was rejected`, { element: 'cmaf' })
  }
}

const aborted = (message: string) => new PlayerError('ABORTED', message, { element: 'cmaf' })

if (import.meta.vitest) {
  const { test, expect, vi, afterEach } = import.meta.vitest
  const { cmafTrack } = await import('../../testing/cmaf')
  afterEach(() => void vi.useRealTimers())
  const spaced = (name: string, keyframeSpacing: number) => cmafTrack(name, { keyframeSpacing })
  const point = (from: CatalogTrack, to: CatalogTrack, group: number, object: number, set = [from, to]) => {
    const { group: g, end } = switchPoint(from, to, new Location(group, object), set, 1n)
    return `${g} after ${end.group}.${end.object ?? '*'}`
  }

  test('switchPoint: MSF 6.3.2 from G₀ = 1, and the next group without keyframeSpacing', () => {
    const [s15, s60, s30] = [spaced('a', 15), spaced('b', 60), spaced('c', 30)]
    // Short to long: frames 0–44 delivered, so the long track starts at frame 60 (its group 2).
    expect(point(s15, s60, 3, 14)).toBe('2 after 4.14')
    // Long to short: frames 0–70 delivered, the short track joins at frame 75 (its group 6).
    expect(point(s60, s15, 2, 10)).toBe('6 after 2.14')
    // Same spacing: the next group, after the whole current one.
    expect(point(s60, spaced('d', 60), 2, 10)).toBe('3 after 2.59')
    // A frame that is a target keyframe hasn't been delivered yet: it's the switch point.
    expect(point(s15, s30, 2, 14)).toBe('2 after 2.14')
    expect(point(cmafTrack('x'), cmafTrack('y'), 7, 3)).toBe('8 after 7.*')
    // One track of the set without keyframeSpacing: not a fast switching set.
    expect(point(s15, s60, 3, 14, [s15, s60, cmafTrack('z')])).toBe('4 after 3.*')
  })

  const { cmafChain } = await import('../../testing/cmaf')
  const { mediaObject } = await import('../../testing/client')
  const { flush } = await import('../../testing/elements')
  const v2 = (group: number, object = 0) => mediaObject(group, object, null, 'v2')

  /** A chain on `set` that delivered group 1, objects 0 and 1; `renditions` hears renditionchange. */
  async function playing(set: CatalogTrack[], cmaf: MediaSourceOptions = {}) {
    const c = await cmafChain(set, { cmaf })
    const renditions: string[] = []
    c.chain.host.player.on('renditionchange', (_, rendition) => renditions.push(rendition.id))
    await flush()
    c.client.last.push(mediaObject(1, 0), mediaObject(1, 1))
    await flush()
    return Object.assign(c, { renditions })
  }

  test('live default: SUBSCRIBE the target from the next group, end the old one at its group end', async () => {
    const c = await playing([cmafTrack('v1'), cmafTrack('v2')])
    let resolved = false
    const switched = c.source.switch('v2').then(() => (resolved = true))
    await flush()
    expect(c.requests()).toEqual(['subscribe v1 next', 'subscribe v2 2.0-', 'update 0 1.0-1'])
    const [old, next] = c.client.requests
    old!.push(mediaObject(1, 2), mediaObject(2, 0)) // the old track's group 2 is past the switch point
    await flush()
    expect(c.requests().at(-1)).toBe('unsubscribe 0')
    expect(resolved).toBe(false) // until the new track's first packet
    next!.push(v2(2, 0))
    await switched
    expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', '1.1', '1.2', 'props track', '2.0'])
    expect(c.renditions).toEqual(['v2'])
    expect(c.source.getRenditions().find((r) => r.active)?.id).toBe('v2')
  })

  test('live fast switch: the target joins at its switch point, the old one ends at its last object', async () => {
    const c = await playing([cmafTrack('v1', { keyframeSpacing: 15 }), cmafTrack('v2', { keyframeSpacing: 60 })])
    void c.source.switch('v2')
    await flush()
    // Frames 0 and 1 delivered: the long track's group 2 starts at frame 60, the old track ends at frame 59.
    expect(c.requests().slice(1)).toEqual(['subscribe v2 2.0-', 'update 0 1.0-4.14'])
  })

  test('switchFrom: one soft SWITCH_FROM; old objects past the switch point are dropped', async () => {
    const c = await playing([cmafTrack('v1'), cmafTrack('v2')], { switchFrom: true })
    const switched = c.source.switch('v2')
    await flush()
    expect(c.requests()).toEqual(['subscribe v1 next', 'switch 0 v2 2.0- fill'])
    const options = c.client.sent<SwitchOptions>('switch')[0]!
    expect(options.switchMode).toBe(SwitchMode.Soft)
    // Both tracks arrive on the original stream.
    c.client.last.push(mediaObject(1, 2), mediaObject(2, 0), v2(2, 0), mediaObject(2, 1), v2(2, 1))
    await switched
    await flush()
    expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', '1.1', '1.2', 'props track', '2.0', '2.1'])
  })

  test('VOD: FETCH the target from the next group, cancel the old fetch at its group end', async () => {
    const c = await playing([cmafTrack('v1', { isLive: false }), cmafTrack('v2', { isLive: false })])
    void c.source.switch('v2')
    await flush()
    const [old, next] = c.client.requests
    old!.push(mediaObject(2, 0))
    next!.push(v2(2, 0))
    await flush()
    expect(c.requests()).toEqual(['fetch v1 0.0-', 'fetch v2 2.0-', 'fetchCancel 0'])
    expect(c.trace().slice(-2)).toEqual(['props track', '2.0'])
  })

  test('unknown, active, and concurrent switches', async () => {
    const c = await playing([cmafTrack('v1'), cmafTrack('v2'), cmafTrack('v3')])
    await expect(c.source.switch('nope')).rejects.toThrow(PlayerError)
    await c.source.switch('v1')
    const first = c.source.switch('v2')
    const latest = c.source.switch('v3')
    await expect(first).rejects.toMatchObject({ code: 'ABORTED' })
    await flush()
    expect(c.requests().slice(1)).toEqual(['subscribe v3 2.0-', 'update 0 1.0-1'])
    c.client.last.push(mediaObject(2, 0, null, 'v3'))
    c.client.requests[0]!.end()
    await latest
  })

  test('a target that fails before its first packet rejects the switch, and the next one runs', async () => {
    vi.useFakeTimers()
    const c = await cmafChain([cmafTrack('v1'), cmafTrack('v2'), cmafTrack('v3')], { reconnectAttempts: 1 })
    await vi.advanceTimersByTimeAsync(0)
    c.client.last.push(mediaObject(1, 0))
    await vi.advanceTimersByTimeAsync(0)
    const failed = c.source.switch('v2')
    failed.catch(() => {})
    await vi.advanceTimersByTimeAsync(0)
    const [old, next] = c.client.requests
    old!.end() // the old track reaches its switch point
    await vi.advanceTimersByTimeAsync(0)
    next!.reset() // and the target fails: the set falls back to v1
    await vi.advanceTimersByTimeAsync(6000)
    c.client.last.push(mediaObject(5, 0))
    await vi.advanceTimersByTimeAsync(0)
    await expect(failed).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
    void c.source.switch('v3')
    await vi.advanceTimersByTimeAsync(0)
    expect(c.requests().at(-2)).toBe('subscribe v3 6.0-')
  })

  test('a switch before any data, or during a seek, restarts on the new track', async () => {
    const c = await cmafChain([cmafTrack('v1'), cmafTrack('v2')])
    await flush()
    const switched = c.source.switch('v2')
    await flush()
    c.client.last.push(v2(4))
    await switched
    expect(c.requests()).toEqual(['subscribe v1 next', 'subscribe v2 next', 'unsubscribe 0'])
  })

  test('selectRenditions: a deselected active rendition switches; none selected stops, a new selection resumes', async () => {
    const c = await playing([
      cmafTrack('v1', { bitrate: 1000 }),
      cmafTrack('v2', { bitrate: 900 }),
      cmafTrack('v3', { bitrate: 5000 }),
    ])
    void c.source.selectRenditions(['v2', 'v3'])
    await flush()
    expect(c.requests().slice(1, 2)).toEqual(['subscribe v2 2.0-'])
    c.client.requests[1]!.push(v2(2))
    c.client.requests[0]!.end()
    await flush()
    await c.source.selectRenditions([])
    await flush()
    expect(c.requests().at(-1)).toBe('unsubscribe 1')
    await c.source.selectRenditions(['v2'])
    await flush()
    c.client.last.push(v2(9))
    await flush()
    expect(c.requests().at(-1)).toBe('subscribe v2 next')
    expect(c.trace().slice(-2)).toEqual(['gap', '9.0'])
    expect(c.failures).toEqual([])
  })

  test('setTracks: a removed active track switches to the closest bitrate, an empty set removes the pad', async () => {
    const c = await playing([cmafTrack('v1', { bitrate: 1000 }), cmafTrack('v2', { bitrate: 900 })])
    c.source.setTracks([cmafTrack('v2', { bitrate: 900 }), cmafTrack('v3', { bitrate: 5000 })])
    await flush()
    expect(c.requests().slice(1, 2)).toEqual(['subscribe v2 2.0-'])
    expect(c.source.getRenditions().map((r) => [r.id, r.selected])).toEqual([
      ['v2', true],
      ['v3', true],
    ])
    c.source.setTracks([])
    expect(c.source.pads).toEqual([])
  })
}
