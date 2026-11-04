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
 * URL ranges as locations (`PLAN.md` › Source Elements): converted once at start, merged,
 * sorted, and clamped to the accessible window (MSF 5.2.46). Locations are written against
 * the set's first track; other tracks map through the MSF 6.3.2 frame index.
 */

import { Location } from '../../../model/common/location'
import type { CatalogTrack, MediaSourceOptions, TimelineReader } from '../../element/interfaces'
import { PlayerError } from '../../error'

/** Inclusive. No object: the whole group. */
export interface End {
  group: bigint
  object?: bigint
}

/** One range to play. No end: open, to the end of the content. */
export interface Span {
  start: Location
  end?: End
  /** FETCH it: VOD, or a closed live range behind the live edge. Otherwise a live history SUBSCRIBE. */
  fetch: boolean
}

/** Whether `location` comes after `end`. */
export const beyond = (location: Location, end: End): boolean =>
  location.group > end.group ||
  (end.object !== undefined && location.group === end.group && location.object > end.object)

/**
 * The URL ranges of `first` (the set's first track) as spans, merged and sorted. A closed
 * live range is fetched when it ends behind the timeline's live edge, or when there's no edge.
 * @throws PlayerError `UNSUPPORTED_SOURCE` for a media time or wallclock range without a timeline.
 */
export function toSpans(
  ranges: Partial<NonNullable<MediaSourceOptions['ranges']>> | undefined,
  first: CatalogTrack,
  timeline: TimelineReader | undefined,
): Span[] {
  const converted: { start: Location; end?: End }[] = [...(ranges?.locationRanges ?? [])]
  const timed = [
    ...(ranges?.mediaTimeRanges ?? []).map((r) => [r, timeline?.timeToLocation.bind(timeline)] as const),
    ...(ranges?.wallclockRanges ?? []).map((r) => [r, timeline?.wallclockToLocation.bind(timeline)] as const),
  ]
  for (const [range, toLocation] of timed) {
    // Estimating would play the wrong content.
    if (!toLocation) throw new PlayerError('UNSUPPORTED_SOURCE', 'A time range needs a media timeline or template')
    const start = toLocation(range.start)
    const end = range.end === undefined ? undefined : toLocation(range.end)
    if (!start || (range.end !== undefined && !end)) {
      throw new PlayerError('UNSUPPORTED_SOURCE', `Range ${range.start}-${range.end ?? ''} is outside the timeline`)
    }
    converted.push({ start: new Location(start.group, 0n), ...(end && { end: { group: end.group } }) })
  }
  const window = accessibleStart(first, timeline)
  const edge = timeline?.range && timeline.timeToLocation(timeline.range.end)
  return merge(converted)
    .filter((r) => !window || !r.end || !beyond(window, r.end))
    .map((r) => ({
      start: window && window.compare(r.start) > 0 ? window : r.start,
      ...(r.end && { end: r.end }),
      // Without a known edge, a closed live range is taken to be history.
      fetch: !first.isLive || (!!r.end && (!edge || beyond(edge, r.end))),
    }))
}

/** Sorted by start, overlapping ranges merged. */
export function merge(ranges: readonly { start: Location; end?: End }[]): { start: Location; end?: End }[] {
  const sorted = [...ranges].sort((a, b) => a.start.compare(b.start))
  const merged: { start: Location; end?: End }[] = []
  for (const range of sorted) {
    const last = merged.at(-1)
    if (last && (!last.end || !beyond(range.start, last.end))) {
      if (!range.end) delete last.end
      else if (last.end && laterEnd(range.end, last.end)) last.end = range.end
    } else merged.push({ ...range })
  }
  return merged
}

const laterEnd = (a: End, b: End): boolean =>
  a.group !== b.group ? a.group > b.group : a.object === undefined || (b.object !== undefined && a.object > b.object)

/** The earliest accessible location (MSF 5.2.46), when the track has a window and a timeline. */
export function accessibleStart(track: CatalogTrack, timeline: TimelineReader | undefined): Location | undefined {
  if (!timeline || track.accessibleWindowTime === undefined) return undefined
  return timeline.wallclockToLocation(Date.now() - track.accessibleWindowTime)
}

/** The span `location` falls in, or the nearest one, and the location clamped into it. */
export function clamp(location: Location, spans: readonly Span[]): { index: number; location: Location } {
  let best = { index: 0, location: spans[0]!.start, distance: Infinity }
  spans.forEach((span, index) => {
    const before = span.start.compare(location) > 0
    const after = span.end && beyond(location, span.end)
    if (!before && !after) best = { index, location, distance: 0 }
    const edge = before ? span.start : after ? new Location(span.end!.group, span.end!.object ?? 0n) : location
    const distance = Math.abs(Number(edge.group - location.group))
    if (distance < best.distance) best = { index, location: edge, distance }
  })
  return { index: best.index, location: best.location }
}

/**
 * `location` of `from` as a location of `to`, through the frame index `(G − G₀) × S + N`
 * (MSF 6.3.2), when their `keyframeSpacing` differs. A start maps to its group's first object.
 */
export function mapLocation(location: Location, from: CatalogTrack, to: CatalogTrack, g0: bigint): Location {
  const [s, t] = [from.keyframeSpacing, to.keyframeSpacing]
  if (!s || !t || s === t) return location
  const frame = (location.group - g0) * BigInt(s) + location.object
  return new Location(g0 + frame / BigInt(t), frame % BigInt(t))
}

/** `end` of `from` as an end of `to`; a whole group ends at its last frame. */
export function mapEnd(end: End, from: CatalogTrack, to: CatalogTrack, g0: bigint): End {
  const [s, t] = [from.keyframeSpacing, to.keyframeSpacing]
  if (!s || !t || s === t) return end
  const mapped = mapLocation(new Location(end.group, end.object ?? BigInt(s - 1)), from, to, g0)
  return { group: mapped.group, object: mapped.object }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafTrack } = await import('../../testing/cmaf')
  const at = (group: number, object = 0) => new Location(group, object)
  const show = (spans: { start: Location; end?: End; fetch?: boolean }[]) =>
    spans.map(
      (s) =>
        `${s.start.group}.${s.start.object}-${s.end ? `${s.end.group}.${s.end.object ?? '*'}` : ''}${s.fetch ? ' fetch' : ''}`,
    )
  const timeline = (end?: number): TimelineReader => ({
    timeToLocation: (ms) => at(1 + Math.floor(ms / 2000)),
    locationToTime: (l) => Number(l.group - 1n) * 2000,
    wallclockToLocation: (ms) => at(1 + Math.floor((ms - 1_000_000) / 2000)),
    range: end === undefined ? undefined : { start: 0, end },
  })

  test('merge: sorted, overlapping ranges joined, an open range absorbs what follows', () => {
    const ranges = [
      { start: at(10), end: { group: 12n } },
      { start: at(1), end: { group: 3n, object: 5n } },
      { start: at(3, 2), end: { group: 4n } },
      { start: at(11) },
      { start: at(20), end: { group: 21n } },
    ]
    expect(show(merge(ranges))).toEqual(['1.0-4.*', '10.0-'])
  })

  test('toSpans: each conversion, fetch behind the edge, and no timeline for a time range', () => {
    const live = cmafTrack('v1')
    const vod = cmafTrack('v1', { isLive: false })
    const locationRanges = [{ start: at(2), end: { group: 3n } }, { start: at(8) }]
    expect(show(toSpans({ locationRanges }, live, undefined))).toEqual(['2.0-3.* fetch', '8.0-'])
    expect(show(toSpans({ locationRanges }, vod, undefined))).toEqual(['2.0-3.* fetch', '8.0- fetch'])
    // The edge is at group 6: a closed range reaching past it is a live history subscribe.
    const mediaTimeRanges = [
      { start: 2000, end: 5000 },
      { start: 9000, end: 13000 },
    ]
    expect(show(toSpans({ mediaTimeRanges }, live, timeline(10000)))).toEqual(['2.0-3.* fetch', '5.0-7.*'])
    const wallclockRanges = [{ start: 1_004_000 }]
    expect(show(toSpans({ wallclockRanges }, live, timeline()))).toEqual(['3.0-'])
    expect(() => toSpans({ mediaTimeRanges }, live, undefined)).toThrow(PlayerError)
  })

  test('toSpans: the accessible window cuts ranges before it', () => {
    const now = Date.now()
    const track = cmafTrack('v1', { accessibleWindowTime: now - 1_010_000 }) // window start at group 6
    const locationRanges = [
      { start: at(1), end: { group: 2n } },
      { start: at(4), end: { group: 9n } },
    ]
    expect(show(toSpans({ locationRanges }, track, timeline()))).toEqual(['6.0-9.* fetch'])
  })

  test('clamp: inside a span, before the first, between two, after the last', () => {
    const spans = merge([
      { start: at(5), end: { group: 8n } },
      { start: at(20), end: { group: 22n, object: 3n } },
    ]).map((s) => ({ ...s, fetch: true }))
    expect(clamp(at(6, 4), spans)).toEqual({ index: 0, location: at(6, 4) })
    expect(clamp(at(1), spans)).toEqual({ index: 0, location: at(5) })
    expect(clamp(at(10), spans)).toEqual({ index: 0, location: at(8) })
    expect(clamp(at(18), spans)).toEqual({ index: 1, location: at(20) })
    expect(clamp(at(30), spans)).toEqual({ index: 1, location: at(22, 3) })
  })

  const { cmafChain, fakeCatalog, twoSecondTimeline } = await import('../../testing/cmaf')
  const { mediaObject } = await import('../../testing/client')
  const { flush } = await import('../../testing/elements')
  const urlRanges = (r: Partial<NonNullable<MediaSourceOptions['ranges']>>) => ({
    ranges: { locationRanges: [], mediaTimeRanges: [], wallclockRanges: [], ...r },
  })

  test('overlapping ranges play merged, in order, with a gap between them and one eos', async () => {
    const locationRanges = [
      { start: at(3), end: { group: 4n } },
      { start: at(2), end: { group: 3n } },
      { start: at(8), end: { group: 8n } },
    ]
    const c = await cmafChain([cmafTrack('v1', { isLive: false })], { cmaf: urlRanges({ locationRanges }) })
    await flush()
    c.client.last.push(mediaObject(2), mediaObject(4))
    c.client.last.end()
    await flush()
    c.client.last.push(mediaObject(8), mediaObject(9)) // past the last range: not written
    await flush()
    expect(c.requests()).toEqual(['fetch v1 2.0-4', 'fetch v1 8.0-8', 'fetchCancel 1'])
    expect(c.trace()).toEqual(['props live,mimeType,track', '2.0', '4.0', 'gap', '8.0', 'eos'])
  })

  test('a live range: history fill, ending at its end; its duration from the timeline', async () => {
    const mediaTimeRanges = [{ start: 4000, end: 7000 }]
    const catalog = fakeCatalog([], twoSecondTimeline(100_000))
    const c = await cmafChain([cmafTrack('v1')], { catalog, cmaf: urlRanges({ mediaTimeRanges }) })
    await flush()
    // Groups 3 to 4 end at 10 s, behind the edge: fetched.
    expect(c.requests()).toEqual(['fetch v1 3.0-4'])
    expect(c.sink.items()[0]).toMatchObject({ properties: { duration: 8 } })
  })

  test('a time range without a timeline is UNSUPPORTED_SOURCE', async () => {
    await expect(
      cmafChain([cmafTrack('v1')], { cmaf: urlRanges({ mediaTimeRanges: [{ start: 0, end: 1000 }] }) }),
    ).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
  })

  test('a seek clamps to the nearest range', async () => {
    const locationRanges = [
      { start: at(2), end: { group: 3n } },
      { start: at(10), end: { group: 12n } },
    ]
    const catalog = fakeCatalog([], twoSecondTimeline())
    const c = await cmafChain([cmafTrack('v1', { isLive: false })], { catalog, cmaf: urlRanges({ locationRanges }) })
    await flush()
    c.client.last.push(mediaObject(2))
    await flush()
    c.seek(15) // group 8, nearer to the second range
    await flush()
    expect(c.requests().at(-1)).toBe('fetch v1 10.0-12')
  })

  test('mapLocation and mapEnd through the frame index, from G₀', () => {
    const [short, long] = [cmafTrack('a', { keyframeSpacing: 15 }), cmafTrack('b', { keyframeSpacing: 60 })]
    // Frame (5 − 1) × 15 + 3 = 63 is group 1 + 63 / 60 = 2 of the long track, object 3.
    expect(mapLocation(at(5, 3), short, long, 1n)).toEqual(at(2, 3))
    expect(mapLocation(at(2, 3), long, short, 1n)).toEqual(at(5, 3))
    expect(mapLocation(at(5, 3), short, cmafTrack('c'), 1n)).toEqual(at(5, 3))
    // Group 2 of the long track ends at frame 119: group 8 of the short one, object 14.
    expect(mapEnd({ group: 2n }, long, short, 1n)).toEqual({ group: 8n, object: 14n })
  })
}
