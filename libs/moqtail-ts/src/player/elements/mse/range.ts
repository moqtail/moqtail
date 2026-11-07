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

/** Duration, live seekable range, and live edge start from the pads' properties. Seconds. */

import type { PadProperties } from '@/player/element/pad'

type Properties = Partial<PadProperties>

/** VOD: the longest `duration`, else the latest timeline end. Undefined leaves it to what's buffered. */
export function vodDuration(pads: readonly Properties[]): number | undefined {
  const durations = pads.flatMap((p) => p.duration ?? [])
  if (durations.length > 0) return Math.max(...durations)
  const ends = pads.flatMap((p) => p.seekable?.end ?? [])
  return ends.length > 0 ? Math.max(...ends) : undefined
}

/**
 * Live: from the timeline start, else the buffered start, to the live edge minus the target
 * latency, so seeking to the end never lands right at the edge (proposal 0007).
 */
export function liveSeekable(
  buffered: TimeRanges,
  pads: readonly Properties[],
): { start: number; end: number } | undefined {
  const ranges = pads.flatMap((p) => p.seekable ?? [])
  const last = buffered.length - 1
  if (ranges.length === 0 && last < 0) return undefined
  const start = ranges.length > 0 ? Math.max(...ranges.map((r) => r.start)) : buffered.start(0)
  const edge = Math.max(...ranges.map((r) => r.end), last >= 0 ? buffered.end(last) : -Infinity)
  return { start, end: Math.max(start, edge - targetLatency(pads)) }
}

/** The seekable end minus 3 group durations (proposal 0007); `NaN` while no group duration is known. */
export function liveEdgeStart(seekableEnd: number, pads: readonly Properties[]): number {
  const duration = groupDuration(pads)
  return duration === undefined ? NaN : seekableEnd - 3 * duration
}

/** `targetLatency` (the catalog's `targetLatency` or `buffers.target`), else 3 group durations. */
function targetLatency(pads: readonly Properties[]): number {
  const latencies = pads.flatMap((p) => p.targetLatency ?? [])
  if (latencies.length > 0) return Math.max(...latencies)
  return 3 * (groupDuration(pads) ?? 0)
}

function groupDuration(pads: readonly Properties[]): number | undefined {
  const durations = pads.flatMap((p) => p.groupDuration ?? [])
  return durations.length > 0 ? Math.max(...durations) : undefined
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { timeRanges } = await import('@/player/testing/media')

  test('vodDuration: the longest duration, else the timeline end', () => {
    expect(vodDuration([{ duration: 4 }, { duration: 6, seekable: { start: 0, end: 9 } }])).toBe(6)
    expect(vodDuration([{ seekable: { start: 0, end: 9 } }, {}])).toBe(9)
    expect(vodDuration([{}])).toBeUndefined()
  })

  test('liveSeekable: timeline start or buffered start, to the edge minus the target latency', () => {
    const buffered = timeRanges([[10, 20]])
    expect(liveSeekable(timeRanges([]), [{}])).toBeUndefined()
    expect(liveSeekable(buffered, [{ targetLatency: 1.5 }])).toEqual({ start: 10, end: 18.5 })
    expect(liveSeekable(buffered, [{ groupDuration: 2 }])).toEqual({ start: 10, end: 14 })
    expect(liveSeekable(buffered, [{ groupDuration: 2, seekable: { start: 0, end: 22 } }])).toEqual({
      start: 0,
      end: 16,
    })
    // Too little to subtract: the range shrinks to its start
    expect(liveSeekable(timeRanges([[10, 11]]), [{ targetLatency: 5 }])).toEqual({ start: 10, end: 10 })
  })

  test('liveEdgeStart: 3 group durations before the seekable end', () => {
    expect(liveEdgeStart(20, [{ groupDuration: 2 }, { groupDuration: 1 }])).toBe(14)
    expect(liveEdgeStart(20, [{}])).toBeNaN()
  })
}
