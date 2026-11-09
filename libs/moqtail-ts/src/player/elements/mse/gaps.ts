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

/** Gap jumping, ported from demo-abr's `MSEBuffer` with its thresholds. Times in seconds. */

/** A gap this small is jumped over right away. */
const IMMEDIATE_SEEK_THRESHOLD = 0.1
/** How close to a range's end playback must be before jumping, and the shortest range worth jumping to. */
const STALL_THRESHOLD = 0.5
/** Default `liveEdgeDelay`: how far behind the end of the last range a live jump lands. */
export const LIVE_EDGE_DELAY = 0.75
/** How often the sink checks for gaps, besides `waiting`, `stalled`, and `discontinuity` events. */
export const GAP_CHECK_INTERVAL_MS = 250

/**
 * Where to jump to get past a gap at `currentTime`, or `undefined` to stay. A range
 * shorter than `liveEdgeDelay` isn't worth jumping to.
 */
export function gapJumpTarget(
  currentTime: number,
  buffered: TimeRanges,
  live: boolean,
  liveEdgeDelay: number,
): number | undefined {
  const index = rangeAt(currentTime, buffered)
  if (index === -1) return rangeAhead(currentTime, buffered, live, liveEdgeDelay)
  const end = buffered.end(index)
  if (end - currentTime > STALL_THRESHOLD) return undefined
  for (let next = index + 1; next < buffered.length; next++) {
    const start = buffered.start(next)
    // Past ranges too short to play from, not only the next one
    if (start - end < IMMEDIATE_SEEK_THRESHOLD) return start
    const length = buffered.end(next) - start
    if (length >= STALL_THRESHOLD && length >= liveEdgeDelay) return start
  }
  return undefined
}

/** WebKit rounds `currentTime`, so a jump to a range's start can land a hair before it. */
const START_TOLERANCE = 0.001

/** Index of the range containing `time`, or -1. */
export function rangeAt(time: number, ranges: TimeRanges): number {
  for (let i = 0; i < ranges.length; i++)
    if (ranges.start(i) - START_TOLERANCE <= time && time <= ranges.end(i)) return i
  return -1
}

/**
 * Outside every range: live goes near the end of the last range, VOD to the start of the next
 * one. Not to a closer edge behind: playback that ran past a range's end (one track ending a
 * little later than the other) would only be sent back to it.
 */
function rangeAhead(
  currentTime: number,
  buffered: TimeRanges,
  live: boolean,
  liveEdgeDelay: number,
): number | undefined {
  if (buffered.length === 0) return undefined
  if (live) return liveEdge(buffered, liveEdgeDelay)
  for (let i = 0; i < buffered.length; i++) if (buffered.start(i) > currentTime) return buffered.start(i)
  return undefined
}

/** `liveEdgeDelay` behind the end of the last range, but not before its start. Needs at least one range. */
export function liveEdge(buffered: TimeRanges, liveEdgeDelay: number): number {
  const last = buffered.length - 1
  return Math.max(buffered.start(last), buffered.end(last) - liveEdgeDelay)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { timeRanges } = await import('@/player/testing/media')

  describe('gapJumpTarget', () => {
    const holes = timeRanges([
      [0, 10],
      [10.05, 10.3],
      [12, 12.2],
      [13, 20],
    ])

    test('stays while playback is not near the end of its range', () => {
      expect(gapJumpTarget(5, holes, false, LIVE_EDGE_DELAY)).toBeUndefined()
      expect(gapJumpTarget(14, holes, true, LIVE_EDGE_DELAY)).toBeUndefined()
      expect(gapJumpTarget(19.8, holes, true, LIVE_EDGE_DELAY)).toBeUndefined() // nothing after the last range
    })

    test('jumps a tiny gap at once, and skips ranges too short to play', () => {
      expect(gapJumpTarget(9.8, holes, false, LIVE_EDGE_DELAY)).toBe(10.05)
      expect(gapJumpTarget(10.2, holes, false, LIVE_EDGE_DELAY)).toBe(13)
    })

    test('outside every range: live goes near the end of the last one, VOD to the next one', () => {
      expect(gapJumpTarget(11, holes, true, LIVE_EDGE_DELAY)).toBe(19.25)
      expect(gapJumpTarget(12.7, holes, false, LIVE_EDGE_DELAY)).toBe(13)
      expect(gapJumpTarget(10.4, holes, false, LIVE_EDGE_DELAY)).toBe(12) // not back to 10.3
      expect(gapJumpTarget(21, holes, false, LIVE_EDGE_DELAY)).toBeUndefined()
      expect(gapJumpTarget(1, timeRanges([]), true, LIVE_EDGE_DELAY)).toBeUndefined()
    })

    test('a time WebKit rounded to just before a range start is in it', () => {
      expect(rangeAt(257257.0463053, timeRanges([[257257.04630533853, 257257.3]]))).toBe(0)
      expect(gapJumpTarget(12.9999, holes, false, LIVE_EDGE_DELAY)).toBeUndefined()
    })
  })
}
