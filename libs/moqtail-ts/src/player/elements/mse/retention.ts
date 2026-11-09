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
 * The default retention policy: keep `backBuffer` behind `currentTime`, cap a VOD forward buffer at `forwardCap`, and make
 * room when an append hits the quota. `buffered` is the source of truth, since `ManagedMediaSource`
 * may evict on its own. Times in seconds.
 */

import type { BufferQueue } from './buffer_queue'

/** Default `backBuffer`. */
export const BACK_BUFFER = 90
/** Default `forwardCap`. */
export const FORWARD_CAP = 60
/** What a quota retry keeps behind `currentTime`: enough to not remove what's about to play. */
export const QUOTA_KEEP = 2

/** End of the last buffered range, or `undefined` when nothing is buffered. */
export function bufferedEnd(buffer: SourceBuffer): number | undefined {
  const ranges = buffer.buffered
  return ranges.length ? ranges.end(ranges.length - 1) : undefined
}

/** Seconds buffered ahead of `currentTime`. */
export function bufferedAhead(buffer: SourceBuffer, currentTime: number): number {
  return (bufferedEnd(buffer) ?? currentTime) - currentTime
}

/** Whether the buffer holds less than `forwardCap` ahead of `currentTime`. */
export function hasRoom(buffer: SourceBuffer, currentTime: number, forwardCap: number): boolean {
  return bufferedAhead(buffer, currentTime) < forwardCap
}

export function isQuotaExceeded(error: unknown): boolean {
  return (error as Error | undefined)?.name === 'QuotaExceededError'
}

/** Removes `[start, end)` clipped to what's buffered; does nothing when that's empty. Run it inside the queue. */
export function removeBuffered(buffer: SourceBuffer, start: number, end: number): void {
  const ranges = buffer.buffered
  if (ranges.length === 0) return
  const from = Math.max(start, ranges.start(0))
  const to = Math.min(end, ranges.end(ranges.length - 1))
  if (to > from) buffer.remove(from, to)
}

/**
 * Appends `data`, then trims everything more than `backBuffer` behind `currentTime`. A
 * `QuotaExceededError` frees the back buffer down to `QUOTA_KEEP` and retries once; when
 * there's nothing to free it rethrows. `keyframes` are group start times, see `groupStart()`.
 */
export async function append(
  queue: BufferQueue,
  data: Uint8Array,
  currentTime: () => number,
  keyframes: readonly number[],
  backBuffer: number,
): Promise<void> {
  try {
    await queue.run((b) => b.appendBuffer(data as Uint8Array<ArrayBuffer>))
  } catch (error) {
    const ranges = queue.buffer.buffered
    const keepFrom = trimEnd(currentTime(), QUOTA_KEEP, keyframes)
    if (!isQuotaExceeded(error) || !ranges.length || ranges.start(0) >= keepFrom) throw error
    await queue.run((b) => removeBuffered(b, 0, keepFrom))
    await queue.run((b) => b.appendBuffer(data as Uint8Array<ArrayBuffer>))
  }
  await queue.run((b) => removeBuffered(b, 0, trimEnd(currentTime(), backBuffer, keyframes)))
}

/**
 * Where a back buffer trim keeping `keep` seconds may end: never past the start of the
 * group being played, because `remove()` also drops frames up to the next keyframe and
 * would take what's playing. `-Infinity`, so nothing is removed, while no group start is known.
 */
function trimEnd(currentTime: number, keep: number, keyframes: readonly number[]): number {
  return Math.min(currentTime - keep, Math.max(...keyframes.filter((k) => k <= currentTime)))
}

/**
 * Where a group that was just appended starts, without parsing the media. Continuing the
 * previous packet, it starts where the buffer ended before (`previousEnd`). Otherwise
 * (first data, or after a seek) it's the start of the last range, which is never later
 * than the group's keyframe, so a guess only ever frees less.
 */
export function groupStart(buffer: SourceBuffer, previousEnd: number | undefined): number {
  const ranges = buffer.buffered
  return previousEnd ?? ranges.start(ranges.length - 1)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { FakeSourceBuffer } = await import('@/player/testing/media')
  const { BufferQueue } = await import('./buffer_queue')
  const setup = (ranges: [number, number][]) => {
    const buffer = new FakeSourceBuffer()
    buffer.ranges = ranges
    return { buffer, queue: new BufferQueue(buffer as unknown as SourceBuffer) }
  }
  const data = new Uint8Array(1)

  describe('retention', () => {
    test('trims everything more than backBuffer behind after an append, never into the playing group', async () => {
      const { buffer, queue } = setup([[0, 40]])
      await append(queue, data, () => 35, [0, 10, 20, 30], 30)
      expect(buffer.ranges).toEqual([[5, 41]])
      await append(queue, data, () => 35, [0, 33], 1) // keeps 1 s, but the group playing at 35 starts at 33
      expect(buffer.ranges).toEqual([[33, 42]])
      expect([hasRoom(queue.buffer, 35, 60), hasRoom(queue.buffer, -20, 60)]).toEqual([true, false])
    })

    test('a quota error frees the back buffer to 2 s behind and retries once', async () => {
      const { buffer, queue } = setup([[0, 10]])
      buffer.fail = 'quota'
      await append(queue, data, () => 8, [0, 2, 4, 6, 9], 30)
      expect(buffer.log).toEqual(['append 1', 'remove 0-6', 'append 1'])
      expect(buffer.ranges).toEqual([[6, 11]])
    })

    test('a quota retry never cuts into the playing group, and needs one to be known', async () => {
      const { buffer, queue } = setup([[0, 10]])
      buffer.fail = 'quota'
      await append(queue, data, () => 8, [0, 5], 30) // the group playing at 8 starts at 5
      expect(buffer.log).toEqual(['append 1', 'remove 0-5', 'append 1'])
      buffer.fail = 'quota'
      await expect(append(queue, data, () => 8, [], 30)).rejects.toMatchObject({ name: 'QuotaExceededError' })
    })

    test('a quota error with nothing to free is an append failure', async () => {
      const { buffer, queue } = setup([[7, 10]])
      buffer.fail = 'quota'
      await expect(append(queue, data, () => 8, [7], 30)).rejects.toMatchObject({ name: 'QuotaExceededError' })
    })
  })
}
