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

/** The `TimelineReader` of a media timeline template (MSF 7.4). Times in milliseconds. */

import { Location } from '@/model/common/location'
import type { MediaTimelineTemplate, TimelineReader } from '@/player/element/interfaces'

/**
 * Entry `n` follows the MSF 7.4.1 formulas. Lookups pick the entry at or before the value
 * asked for. Without wallclock values (`startWallclock` 0, as for VOD) wallclock lookups and
 * `range` are undefined. Otherwise `range` ends at the entry the current wallclock time falls
 * in, the live edge, and starts at the oldest entry inside `accessibleWindowTime` (MSF 5.2.46).
 */
export class TemplateTimeline implements TimelineReader {
  constructor(
    readonly template: MediaTimelineTemplate,
    readonly accessibleWindowTime?: number,
  ) {}

  timeToLocation(mediaTimeMs: number): Location | undefined {
    const { startMediaTime, deltaMediaTime } = this.template
    return this.#location(index(mediaTimeMs, startMediaTime, deltaMediaTime))
  }

  wallclockToLocation(wallclockMs: number): Location | undefined {
    const { startWallclock, deltaWallclock } = this.template
    return startWallclock > 0 ? this.#location(index(wallclockMs, startWallclock, deltaWallclock)) : undefined
  }

  locationToTime(location: Location): number | undefined {
    const [startGroup, startObject] = this.template.startLocation
    const [deltaGroup, deltaObject] = this.template.deltaLocation
    let n =
      deltaGroup > 0
        ? index(Number(location.group), startGroup, deltaGroup)
        : index(Number(location.object), startObject, deltaObject)
    if (n !== undefined && this.#location(n)!.compare(location) > 0) n--
    return n === undefined || n < 0 ? undefined : this.#mediaTime(n)
  }

  get range(): { start: number; end: number } | undefined {
    const { startWallclock, deltaWallclock } = this.template
    if (startWallclock <= 0) return undefined
    const edge = index(Date.now(), startWallclock, deltaWallclock)
    if (edge === undefined) return undefined
    const window = this.accessibleWindowTime
    const threshold = window === undefined ? startWallclock : Date.now() - window
    const oldest = deltaWallclock > 0 ? Math.ceil((threshold - startWallclock) / deltaWallclock) : 0
    return { start: this.#mediaTime(Math.min(Math.max(0, oldest), edge)), end: this.#mediaTime(edge) }
  }

  #location(n: number | undefined): Location | undefined {
    if (n === undefined) return undefined
    const [startGroup, startObject] = this.template.startLocation
    const [deltaGroup, deltaObject] = this.template.deltaLocation
    return new Location(startGroup + n * deltaGroup, startObject + n * deltaObject)
  }

  #mediaTime(n: number): number {
    return this.template.startMediaTime + n * this.template.deltaMediaTime
  }
}

/** The entry at or before `value` on an axis starting at `start` and growing by `delta`. */
function index(value: number, start: number, delta: number): number | undefined {
  if (value < start) return undefined
  return delta > 0 ? Math.floor((value - start) / delta) : 0
}

if (import.meta.vitest) {
  const { test, expect, vi, afterEach } = import.meta.vitest
  // MSF 5.6.10 and 7.4.1: [0, 2002, [0, 0], [1, 0], 1759924158381, 2002]
  const example: MediaTimelineTemplate = {
    startMediaTime: 0,
    deltaMediaTime: 2002,
    startLocation: [0, 0],
    deltaLocation: [1, 0],
    startWallclock: 1759924158381,
    deltaWallclock: 2002,
  }

  afterEach(() => void vi.useRealTimers())

  test('the MSF 7.4.1 example: entries from media time, location, and wallclock', () => {
    const r = new TemplateTimeline(example)
    expect(r.timeToLocation(4004)).toEqual(new Location(2n, 0n))
    expect(r.timeToLocation(6005)).toEqual(new Location(2n, 0n))
    expect(r.timeToLocation(-1)).toBeUndefined()
    expect(r.locationToTime(new Location(3n, 7n))).toBe(6006)
    expect(r.wallclockToLocation(1759924158381 + 2002 * 10 + 1)).toEqual(new Location(10n, 0n))
    // Objects advancing within one group
    const objects = new TemplateTimeline({ ...example, deltaLocation: [0, 30], startWallclock: 0 })
    expect([objects.locationToTime(new Location(0n, 61n)), objects.timeToLocation(4004)]).toEqual([
      4004,
      new Location(0n, 60n),
    ])
    expect([objects.wallclockToLocation(Date.now()), objects.range]).toEqual([undefined, undefined])
  })

  test('range: open at the live edge, the start inside the accessible window', () => {
    vi.useFakeTimers({ now: 1759924158381 + 2002 * 100 + 5 })
    expect(new TemplateTimeline(example).range).toEqual({ start: 0, end: 200200 })
    expect(new TemplateTimeline(example, 10_000).range).toEqual({ start: 2002 * 96, end: 200200 })
  })
}
