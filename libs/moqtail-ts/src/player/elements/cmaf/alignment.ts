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
 * Group alignment in a fast switching set (draft-ugur-moq-fast-switching). The publisher numbers
 * every track on one grid, the Alignment Unit `Gm` (the set's smallest `keyframeSpacing`): a
 * track with spacing `S` has groups `0, k, 2k, …` with `k = S / Gm`. A frame number, 0-based like
 * group and object IDs, means the same media position on every track of the set.
 */

import { Location } from '@/model/common/location'
import type { CatalogTrack } from '@/player/element/interfaces'
import type { End } from './ranges'

/** `Gm`, when every track of `set` has a `keyframeSpacing` that's a multiple of the smallest. */
export function alignmentUnit(set: readonly CatalogTrack[]): bigint | undefined {
  const spacings = set.map((t) => t.keyframeSpacing ?? 0)
  if (spacings.length === 0 || spacings.some((s) => s <= 0)) return undefined
  const gm = Math.min(...spacings)
  return spacings.every((s) => s % gm === 0) ? BigInt(gm) : undefined
}

/** Every track has a spacing, but they don't share a grid: the set breaks the draft's publisher rules. */
export const misaligned = (set: readonly CatalogTrack[]): boolean =>
  set.length > 0 && set.every((t) => (t.keyframeSpacing ?? 0) > 0) && alignmentUnit(set) === undefined

/** Group IDs between two groups of `track`: `k`, or 1 outside a fast switching set. */
export function groupStep(track: CatalogTrack, set: readonly CatalogTrack[]): bigint {
  const gm = alignmentUnit(set)
  return gm ? BigInt(track.keyframeSpacing!) / gm : 1n
}

/** Seconds per Group ID from the catalog (`Gm` frames), when the set is aligned and `track` has a frame rate. */
export function groupDuration(track: CatalogTrack, set: readonly CatalogTrack[]): number | undefined {
  const gm = alignmentUnit(set)
  return gm && track.framerate ? Number(gm) / track.framerate : undefined
}

/** The frame number of `location` (draft › Frame Numbers). */
export const frameNumber = (location: Location, gm: bigint): bigint => location.group * gm + location.object

/** Where frame `frame` is on a track with spacing `s`: the group on its grid that contains it. */
export function locationOf(frame: bigint, s: bigint, gm: bigint): Location {
  const index = frame / s
  return new Location(index * (s / gm), frame - index * s)
}

/** The first switch point at or after frame `frame` on a track with spacing `t` (draft › Switch Points). */
export const targetGroup = (frame: bigint, t: bigint, gm: bigint): bigint => (t / gm) * ((frame + t - 1n) / t)

/**
 * Where a switch from `from` to `to` happens, after `last` (the last delivered location of
 * `from`): the target's first group, and the old track's last frame before it. The group of
 * that frame is the draft's End Group; it also covers spacings that don't nest, where the
 * draft's "group already being received" would leave a gap. Outside a fast switching set: the next group.
 */
export function switchPoint(
  from: CatalogTrack,
  to: CatalogTrack,
  last: Location,
  set: readonly CatalogTrack[],
): { group: bigint; end: End } {
  const gm = alignmentUnit(set)
  if (!gm) return { group: last.group + 1n, end: { group: last.group } }
  const group = targetGroup(frameNumber(last, gm) + 1n, BigInt(to.keyframeSpacing!), gm)
  const end = locationOf(group * gm - 1n, BigInt(from.keyframeSpacing!), gm)
  return { group, end: { group: end.group, object: end.object } }
}

/** The draft's Distance: frames from the first one not delivered (`last` + 1) to `to`'s next switch point. */
export function switchDistance(to: CatalogTrack, last: Location, set: readonly CatalogTrack[]): bigint {
  const gm = alignmentUnit(set)
  if (!gm) return 0n
  const next = frameNumber(last, gm) + 1n
  return targetGroup(next, BigInt(to.keyframeSpacing!), gm) * gm - next
}

/** `location` of `from` as a location of `to`: the group on `to`'s grid holding the same frame. */
export function mapLocation(
  location: Location,
  from: CatalogTrack,
  to: CatalogTrack,
  set: readonly CatalogTrack[],
): Location {
  const gm = alignmentUnit(set)
  if (!gm || from.keyframeSpacing === to.keyframeSpacing) return location
  return locationOf(frameNumber(location, gm), BigInt(to.keyframeSpacing!), gm)
}

/** `end` of `from` as an end of `to`; a whole group ends at its last frame. */
export function mapEnd(end: End, from: CatalogTrack, to: CatalogTrack, set: readonly CatalogTrack[]): End {
  if (!alignmentUnit(set) || from.keyframeSpacing === to.keyframeSpacing) return end
  const last = new Location(end.group, end.object ?? BigInt(from.keyframeSpacing! - 1))
  const mapped = mapLocation(last, from, to, set)
  return { group: mapped.group, object: mapped.object }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafTrack } = await import('@/player/testing/cmaf')
  const spaced = (name: string, keyframeSpacing: number) => cmafTrack(name, { keyframeSpacing })
  const at = (group: number, object = 0) => new Location(group, object)

  test("the draft's worked examples: frame 405, its switch points, and both End Group cases", () => {
    // Gm = 30; 720p-long (120) at object 45 of group 12 is frame 405.
    expect(frameNumber(at(12, 45), 30n)).toBe(405n)
    expect([30n, 60n, 120n].map((t) => targetGroup(405n, t, 30n))).toEqual([14n, 14n, 16n])
    const set = [spaced('short', 30), spaced('medium', 60), spaced('long', 120), spaced('three', 90)]
    const [short, medium, long, three] = set as [CatalogTrack, CatalogTrack, CatalogTrack, CatalogTrack]
    // Frame 405 is the first one not delivered, so the last one is 404: object 44 of group 12.
    expect([short, medium, long].map((t) => switchDistance(t, at(12, 44), set))).toEqual([15n, 15n, 75n])
    // T <= S: the old track ends with the group it's sending.
    expect(switchPoint(long, medium, at(12, 44), set)).toEqual({ group: 14n, end: { group: 12n, object: 59n } })
    // T > S: S = 60 sending frame 120 (its group 4), T = 90 switches at frame 180, so End Group 4.
    expect(switchPoint(medium, three, at(4, 0), set)).toEqual({ group: 6n, end: { group: 4n, object: 59n } })
  })

  test('ch07_all: 500 ms, 2 s, and 6 s GoPs at 24 fps step by 1, 4, and 12', () => {
    const set = [spaced('6s', 144), spaced('2s', 48), spaced('500ms', 12)]
    const [six, two, half] = set as [CatalogTrack, CatalogTrack, CatalogTrack]
    expect([alignmentUnit(set), groupStep(six, set), groupStep(two, set)]).toEqual([12n, 12n, 4n])
    expect(groupDuration({ ...six, framerate: 24 }, set)).toBe(0.5)
    // At 6 s group 336, object 30: frames up to 336 × 12 + 30 are delivered.
    expect(switchPoint(six, two, at(336, 30), set)).toEqual({ group: 340n, end: { group: 336n, object: 47n } })
    expect(switchPoint(six, half, at(336, 30), set)).toEqual({ group: 339n, end: { group: 336n, object: 35n } })
    // From 500 ms up to 6 s: the old track plays to the frame before group 348.
    expect(switchPoint(half, six, at(339, 3), set)).toEqual({ group: 348n, end: { group: 347n, object: 11n } })
  })

  test('mapping round trips, and a set off the grid maps nothing and switches at the next group', () => {
    const set = [spaced('a', 12), spaced('b', 48)]
    const [a, b] = set as [CatalogTrack, CatalogTrack]
    expect(mapLocation(at(6, 5), a, b, set)).toEqual(at(4, 29))
    expect(mapLocation(at(4, 29), b, a, set)).toEqual(at(6, 5))
    expect(mapEnd({ group: 4n }, b, a, set)).toEqual({ group: 7n, object: 11n })
    expect(mapEnd({ group: 7n, object: 11n }, a, b, set)).toEqual({ group: 4n, object: 47n })

    const off = [spaced('a', 12), spaced('c', 30)]
    expect([alignmentUnit(off), misaligned(off), misaligned([a, cmafTrack('z')])]).toEqual([undefined, true, false])
    expect(mapLocation(at(6, 5), a, off[1]!, off)).toEqual(at(6, 5))
    expect(switchPoint(a, off[1]!, at(6, 5), off)).toEqual({ group: 7n, end: { group: 6n } })
  })
}
