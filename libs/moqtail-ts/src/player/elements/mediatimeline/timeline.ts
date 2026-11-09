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
 * The media timeline element (MSF 7): keeps the explicit entries of a media timeline track
 * and translates between media time, wallclock time, and locations. Times in milliseconds.
 */

import type { Location } from '@/model/common/location'
import { SourceElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { toLocation } from '@/player/element/event_timeline'
import type { TimelineReader } from '@/player/element/interfaces'
import { DEFAULT_CATALOG_TIMEOUT_MS, MEDIA_PRIORITY } from '@/player/constants'
import { PlayerError } from '@/player/error'
import { handledResolvers, within } from '@/player/util/async'
import { parseJsonArray } from '@/player/util/json'
import { TrackReader } from '@/player/util/track_reader'

/** One explicit entry (MSF 7.1.1). `wallclock` is 0 when unknown, as for VOD. */
export interface TimelineEntry {
  mediaTime: number
  location: Location
  wallclock: number
}

/** Reads a media timeline track and answers lookups over its entries. */
export class MediaTimelineElement extends SourceElement {
  /** Sorted by media time. */
  #entries: TimelineEntry[] = []
  readonly #reader: TrackReader
  readonly #first = handledResolvers()

  constructor(context: ElementContext) {
    super(context)
    const { namespace, nameBytes, name } = context.tracks[0]!
    const warn = (message: string, cause?: unknown) =>
      context.player.emit('warning', new PlayerError('TRACK_SKIPPED', message, { element: 'mediatimeline', cause }))
    // MSF 7.3: object 0 holds every accessible entry, later objects only new ones
    const received = (payload: Uint8Array, base: TimelineEntry[]) => {
      const { entries, dropped } = parseEntries(payload)
      if (dropped > 0) warn(`Dropped ${dropped} malformed entr(ies) of media timeline "${name}"`)
      this.#entries = [...base, ...entries].sort((a, b) => a.mediaTime - b.mediaTime)
    }
    this.#reader = new TrackReader(context, {
      track: { namespace, name: nameBytes },
      priority: MEDIA_PRIORITY,
      incremental: true,
      onIndependent: (payload) => {
        received(payload, [])
        this.#first.resolve()
      },
      onIncremental: (payload) => received(payload, this.#entries),
      onStale: (reason) => {
        this.#first.reject(reason)
        warn(`Media timeline "${name}" went stale: ${reason.message}`, reason)
      },
    })
  }

  /**
   * Subscribes and waits for the first independent timeline, so lookups work once it resolves.
   * @throws PlayerError when the subscription is rejected or no timeline arrives in time.
   */
  async start(): Promise<void> {
    const timeoutMs = this.context.player.options.catalogTimeoutMs ?? DEFAULT_CATALOG_TIMEOUT_MS
    try {
      await this.#reader.start()
      await within(this.#first.promise, timeoutMs, this.context.signal, () => {
        return new PlayerError('TRACK_SKIPPED', `No media timeline within ${timeoutMs} ms`, {
          element: 'mediatimeline',
        })
      })
    } catch (error) {
      await this.#reader.stop()
      throw error
    }
  }

  override async destroy(): Promise<void> {
    await this.#reader.stop()
    await super.destroy()
  }

  /** Lookups over the current entries, the range limited by a track's `accessibleWindowTime`. */
  reader(accessibleWindowTime?: number): TimelineReader {
    return new EntriesTimeline(() => this.#entries, accessibleWindowTime)
  }
}

/** The records of an MSF 7.1.1 document; malformed ones are counted, not kept. */
export function parseEntries(payload: Uint8Array): { entries: TimelineEntry[]; dropped: number } {
  const json = parseJsonArray(payload, 'A media timeline must be a JSON array (MSF 7.1.1)')
  const entries = json.flatMap((record: unknown) => {
    if (!Array.isArray(record) || record.length !== 3) return []
    const [mediaTime, at, wallclock] = record
    const location = toLocation(at)
    const valid = location && Number.isFinite(mediaTime) && Number.isFinite(wallclock)
    return valid ? [{ mediaTime: mediaTime as number, location, wallclock: wallclock as number }] : []
  })
  return { entries, dropped: json.length - entries.length }
}

/**
 * Lookups pick the entry with the greatest value at or below the one asked for. Wallclock
 * lookups skip entries without one. `range` runs from the first to the last media time, the
 * start raised to the oldest entry inside the window: by wallclock (MSF 5.2.46, 11.6), or by
 * media time when the timeline has no wallclock values, as 5.2.46 allows for constant-rate content.
 */
export class EntriesTimeline implements TimelineReader {
  /** `entries` is read on every lookup, so updates to the timeline apply at once. */
  constructor(
    readonly entries: () => readonly TimelineEntry[],
    readonly accessibleWindowTime?: number,
  ) {}

  timeToLocation(mediaTimeMs: number): Location | undefined {
    return this.#lastWhere((e) => e.mediaTime <= mediaTimeMs)
  }

  wallclockToLocation(wallclockMs: number): Location | undefined {
    return this.#lastWhere((e) => e.wallclock > 0 && e.wallclock <= wallclockMs)
  }

  locationToTime(location: Location): number | undefined {
    // Locations need not grow with media time (MSF 7.1), so this one searches all entries
    let best: TimelineEntry | undefined
    for (const e of this.entries()) {
      if (e.location.compare(location) <= 0 && (!best || e.location.compare(best.location) > 0)) best = e
    }
    return best?.mediaTime
  }

  timeToWallclock(mediaTimeMs: number): number | undefined {
    const entry = this.entries().findLast((e) => e.mediaTime <= mediaTimeMs)
    return entry && entry.wallclock > 0 ? entry.wallclock + mediaTimeMs - entry.mediaTime : undefined
  }

  get range(): { start: number; end: number } | undefined {
    const list = this.entries()
    const end = list.at(-1)
    const window = this.accessibleWindowTime
    if (!end) return undefined
    if (window === undefined) return { start: list[0]!.mediaTime, end: end.mediaTime }
    const inside =
      end.wallclock > 0
        ? list.find((e) => e.wallclock >= Date.now() - window)
        : list.find((e) => e.mediaTime >= end.mediaTime - window)
    return inside && { start: inside.mediaTime, end: end.mediaTime }
  }

  // Entries are sorted by media time, which wallclock follows
  #lastWhere(ok: (e: TimelineEntry) => boolean): Location | undefined {
    return this.entries().findLast(ok)?.location
  }
}

if (import.meta.vitest) {
  const { describe, test, expect, vi, afterEach } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const text = (json: unknown) => new TextEncoder().encode(JSON.stringify(json))
  // MSF 7.1.1
  const example = [
    [0, [0, 0], 1759924158381],
    [2002, [1, 0], 1759924160383],
    [4004, [2, 0], 1759924162385],
    [6006, [3, 0], 1759924164387],
    [8008, [4, 0], 1759924166389],
  ]
  const reader = (json: unknown, window?: number) => new EntriesTimeline(() => parseEntries(text(json)).entries, window)

  afterEach(() => void vi.useRealTimers())

  describe('media timeline entries', () => {
    test('the MSF 7.1.1 example: the containing entry for each lookup', () => {
      const r = reader(example)
      expect(r.timeToLocation(5000)).toEqual(new Location(2n, 0n))
      expect(r.timeToLocation(-1)).toBeUndefined()
      expect(r.wallclockToLocation(1759924160383)).toEqual(new Location(1n, 0n))
      expect(r.timeToWallclock(2500)).toBe(1759924160383 + 498)
      expect(r.locationToTime(new Location(3n, 5n))).toBe(6006)
      expect(r.range).toEqual({ start: 0, end: 8008 })
    })

    test('malformed entries are dropped, a document that is no array throws', () => {
      const parsed = parseEntries(text([...example.slice(0, 2), [1, [1], 0], [1, [0, 0]], 'x']))
      expect([parsed.entries.length, parsed.dropped]).toEqual([2, 3])
      expect(() => parseEntries(text({}))).toThrow(/MSF 7.1.1/)
    })

    test('the accessible window: by wallclock, or by media time without wallclock values', () => {
      vi.useFakeTimers({ now: 1759924166389 })
      expect(reader(example, 4500).range).toEqual({ start: 4004, end: 8008 })
      expect(reader(example, 0).range).toEqual({ start: 8008, end: 8008 })
      vi.setSystemTime(1759924166389 + 60_000)
      expect(reader(example, 4500).range).toBeUndefined() // nothing left inside the window
      const vod = example.map(([time, at]) => [time, at, 0])
      expect(reader(vod, 4500).range).toEqual({ start: 4004, end: 8008 })
      expect(reader(vod).wallclockToLocation(Date.now())).toBeUndefined()
    })
  })
}
