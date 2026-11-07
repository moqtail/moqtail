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
 * Seeking: the media time of received groups, and a seek target
 * as a location and the request that serves it.
 */

import { Location } from '@/model/common/location'
import type { CatalogTrack, TimelineReader } from '@/player/element/interfaces'
import type { Target } from './delivery'
import { accessibleStart } from './ranges'

type Point = { group: bigint; time: number }

/** The media time of the first object of each group delivered, per track: the observed group duration. */
export class GroupTimes {
  readonly #tracks = new Map<string, { first: Point; last: Point }>()

  record(track: string, group: bigint, time: number): void {
    const known = this.#tracks.get(track)
    if (!known) this.#tracks.set(track, { first: { group, time }, last: { group, time } })
    else if (group > known.last.group) known.last = { group, time }
  }

  /** Seconds per group, once two groups were seen. */
  groupDuration(track: string): number | undefined {
    const known = this.#tracks.get(track)
    if (!known || known.last.group === known.first.group) return undefined
    return (known.last.time - known.first.time) / Number(known.last.group - known.first.group)
  }

  /**
   * The group estimated to contain `mediaTime` (seconds), on the track's grid: its groups are
   * `step` Group IDs apart (`groupStep`), and `duration` is seconds per Group ID.
   */
  estimate(track: string, mediaTime: number, step = 1n, duration = this.groupDuration(track)): Location | undefined {
    const last = this.#tracks.get(track)?.last
    if (!duration || !last) return undefined
    const group = last.group + step * BigInt(Math.floor((mediaTime - last.time) / (duration * Number(step))))
    return new Location(group < 0n ? 0n : group, 0n)
  }
}

/**
 * The group containing `mediaTime` (seconds): from the timeline, else estimated from the data
 * received so far, no earlier than the accessible window (MSF 5.2.46, 7).
 */
export function locate(
  mediaTime: number,
  track: CatalogTrack,
  timeline: TimelineReader | undefined,
  estimate: (mediaTime: number) => Location | undefined,
): Location | undefined {
  const location = timeline?.timeToLocation(mediaTime * 1000) ?? estimate(mediaTime)
  if (!location) return undefined
  const window = accessibleStart(track, timeline)
  return window && window.compare(location) > 0 ? window : new Location(location.group, 0n)
}

/**
 * The request for a seek to `location`: a FETCH for VOD or a fetched range, else one SUBSCRIBE:
 * from the next object, filled from the target, when it's behind the live edge (`last`), else
 * from the next group.
 */
export function seekTarget(location: Location, fetch: boolean, last: Location | undefined): Target {
  if (fetch) return { fetch, start: location }
  if (last && location.group <= last.group) return { fetch, start: location, fill: true }
  return { fetch }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafTrack } = await import('@/player/testing/cmaf')

  test('GroupTimes: estimates from the observed group duration, per track', () => {
    const times = new GroupTimes()
    expect(times.estimate('v1', 5)).toBeUndefined()
    times.record('v1', 3n, 4)
    expect(times.estimate('v1', 5)).toBeUndefined() // one group says nothing about duration
    times.record('v1', 5n, 8)
    times.record('v1', 4n, 6) // an older group doesn't move the latest point
    expect(times.groupDuration('v1')).toBe(2)
    expect(times.estimate('v1', 13)).toEqual(new Location(7n, 0n))
    expect(times.estimate('v1', 0)).toEqual(new Location(1n, 0n))
    expect(times.estimate('v2', 13)).toBeUndefined()
    // A 6 s track of a 500 ms grid (groups 0, 12, 24, …): rounded down onto its grid
    const spaced = new GroupTimes()
    spaced.record('v1', 12n, 6)
    expect(spaced.estimate('v1', 170, 12n)).toBeUndefined() // one group: no duration observed
    expect(spaced.estimate('v1', 170, 12n, 0.5)).toEqual(new Location(336n, 0n))
    spaced.record('v1', 24n, 12)
    expect(spaced.estimate('v1', 170, 12n)).toEqual(new Location(336n, 0n))
  })

  test('locate: the timeline first, the containing group, clamped to the accessible window', () => {
    const timeline: TimelineReader = {
      timeToLocation: (ms) => new Location(1 + Math.floor(ms / 2000), 7),
      locationToTime: () => undefined,
      wallclockToLocation: () => undefined,
      range: { start: 18_000, end: 30_000 },
    }
    const none = () => undefined
    expect(locate(4, cmafTrack('v1'), timeline, none)).toEqual(new Location(3n, 0n))
    expect(locate(4, cmafTrack('v1', { accessibleWindowTime: 12_000 }), timeline, none)).toEqual(new Location(10n, 7n))
    expect(locate(4, cmafTrack('v1'), undefined, () => new Location(9n, 0n))).toEqual(new Location(9n, 0n))
    expect(locate(4, cmafTrack('v1'), undefined, none)).toBeUndefined()
  })

  const { cmafChain, fakeCatalog, timedObject, twoSecondTimeline } = await import('@/player/testing/cmaf')
  const { mediaObject, refuse } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')
  const timeline = twoSecondTimeline()

  test('VOD: cancel the fetch, then FETCH from the containing group, answered right before its first packet', async () => {
    const c = await cmafChain([cmafTrack('v1', { isLive: false })], { catalog: fakeCatalog([], timeline) })
    await flush()
    c.client.last.push(mediaObject(1))
    await flush()
    c.seek(5) // group 3
    await flush()
    c.client.last.push(mediaObject(3))
    await flush()
    expect(c.requests()).toEqual(['fetch v1 0.0-', 'fetchCancel 0', 'fetch v1 3.0-'])
    expect(c.trace()).toEqual(['props live,mimeType,track', '1.0', 'seek', '3.0'])
  })

  test('live: behind the edge one filled SUBSCRIBE, then the old one ends; at the edge from the next group', async () => {
    const c = await cmafChain([cmafTrack('v1')], { catalog: fakeCatalog([], timeline) })
    await flush()
    c.client.last.push(mediaObject(5), mediaObject(6))
    await flush()
    c.seek(5)
    await flush()
    c.seek(60) // group 31, past the edge
    await flush()
    c.client.last.push(mediaObject(40))
    await flush()
    expect(c.requests()).toEqual([
      'subscribe v1 next',
      'subscribe v1 nextobject fill 3.0-',
      'unsubscribe 0',
      'subscribe v1 next',
      'unsubscribe 1',
    ])
    expect(c.trace().slice(-3)).toEqual(['seek', 'seek', '40.0'])
  })

  test('superseded seeks are answered in order before the latest one, which alone is requested', async () => {
    const c = await cmafChain([cmafTrack('v1', { isLive: false })], { catalog: fakeCatalog([], timeline) })
    await flush()
    c.client.last.push(mediaObject(1))
    await flush()
    c.seek(5)
    c.seek(9)
    c.seek(13) // group 7
    await flush()
    c.client.last.push(mediaObject(7))
    await flush()
    expect(c.requests().filter((r) => r.startsWith('fetch '))).toEqual(['fetch v1 0.0-', 'fetch v1 7.0-'])
    expect(c.trace().slice(1)).toEqual(['1.0', 'seek', 'seek', 'seek', '7.0'])
  })

  test('without a timeline, the observed group duration places the seek', async () => {
    const c = await cmafChain([cmafTrack('v1', { isLive: false, timescale: 1000 })])
    await flush()
    c.client.last.push(timedObject(1, 0), timedObject(2, 2), timedObject(3, 4))
    await flush()
    c.seek(11)
    await flush()
    expect(c.requests().at(-1)).toBe('fetch v1 6.0-')
  })

  test('a rejected seek is answered as failed, then the rendition falls back', async () => {
    const c = await cmafChain([cmafTrack('v1', { isLive: false })], { catalog: fakeCatalog([], timeline) })
    await flush()
    c.client.last.push(mediaObject(1))
    await flush()
    c.client.respond = () => refuse()
    c.seek(5)
    await flush()
    expect(c.trace().slice(-1)).toEqual(['seek failed'])
    expect(c.source.pads).toEqual([]) // no rendition left
  })

  test('a seek during a lost session completes after the reconnect', async () => {
    const c = await cmafChain([cmafTrack('v1', { isLive: false })], { catalog: fakeCatalog([], timeline) })
    await flush()
    c.client.last.push(mediaObject(1))
    await flush()
    c.lose()
    c.seek(5)
    await flush()
    c.reconnected()
    await flush()
    c.client.last.push(mediaObject(3))
    await flush()
    expect(c.requests()).toEqual(['fetch v1 3.0-'])
    expect(c.trace().slice(-2)).toEqual(['seek', '3.0'])
  })

  test('seekTarget: VOD fetch, live history fill, live edge from the next group', () => {
    const at = new Location(4n, 0n)
    expect(seekTarget(at, true, undefined)).toEqual({ fetch: true, start: at })
    expect(seekTarget(at, false, new Location(9n, 3n))).toEqual({ fetch: false, start: at, fill: true })
    expect(seekTarget(at, false, new Location(3n, 3n))).toEqual({ fetch: false })
  })
}
