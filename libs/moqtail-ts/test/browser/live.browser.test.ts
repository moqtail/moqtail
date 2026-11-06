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
 * Live playback and live seeking. Live locations move, so these check distances, not locations.
 *
 * Fast switching (draft-ugur-moq-fast-switching), when `keyframeSpacing` is given and there are
 * alternatives: the player joins on the shortest GoP and always climbs to the longest, the most
 * efficient. `keyframeSpacing` is MSF-internal, so renditions collapse: each quality is one
 * rendition with its bitrate and resolution, and a climb up the GoP ladder is no rendition change.
 */

import { Location } from '@/model/common/location'
import { ch07, ch07_all, describe, expect, test, warmUp } from './harness/test'

describe('live', () => {
  warmUp(ch07, ch07_all)

  test('joins at the next group near the live edge, where the template places it, and plays smoothly', async ({
    open,
  }) => {
    const s = await open(ch07.uri())
    expect(s.stateTrace()).toBe('connecting > ready > playing')
    const playing = s.mark()
    await s.played(3)
    s.since(playing).expectSmooth()
    expect(s.latency()).toBeLessThan(2 * ch07.gop!)
    expect(s.requestsOf('video').map((r) => r.kind)).toEqual(['subscribe'])
    for (const role of ['video', 'audio'] as const) {
      // From the next group: the warm relay knows where the current one is. Objects of a group may
      // arrive out of order, so the earliest location counts, not the first to arrive.
      const group = s.groups(role)[0]!
      const objects = s.objectsOf(role).filter((o) => o.group === group)
      expect(Math.min(...objects.map((o) => Number(o.object))), `first ${role} object of group ${group}`).toBe(0)
      // A publisher whose live timestamps drift leaves the template out on live.
      const timeline = s.timelineOf(role)
      if (!timeline) continue
      // The buffer starts at the first group received, where the template should put it.
      const start = timeline.locationToTime(new Location(group, 0n))! / 1000
      expect(s.buffered(role)[0]![0], `${role} group ${group}`).toBeCloseTo(start, 1)
    }
  })

  // Just before what's buffered: a filled SUBSCRIBE that needs only a few seconds of relay and publisher cache.
  test('seeking a group before the buffer resumes there', async ({ open }) => {
    const s = await open(ch07.uri())
    await s.played(2)
    const seek = s.mark()
    // Mid-group, so the template's offset to the media can't put it in the group before.
    const target = s.buffered()[0]![0] - ch07.gop! / 2
    await s.seek(target)
    await s.played(1)
    const after = s.since(seek).newRequests()
    expect(after.requestsOf('video').map((r) => r.kind)).toEqual(['subscribe'])
    // The fill starts at the group holding the target (tolerating the template's offset to the media).
    expect(after.groups('video')[0]! - s.groupAt(target)).toBeLessThanOrEqual(0n)
    const range = s.buffered('video').find(([start, end]) => start <= target && target < end)
    expect(range, `video buffered at ${target.toFixed(2)}`).toBeDefined()
    expect(target - range![0]).toBeLessThan(ch07.gop!)
    expect(s.video.currentTime).toBeGreaterThan(target)
    const resumed = s.mark()
    await s.played(2)
    s.since(resumed).expectSmooth()
  })

  test('ch07_all: one rendition per quality; joins on its shortest GoP and climbs to the longest', async ({ open }) => {
    const s = await open(ch07_all.uri())
    const playing = s.mark()
    // keyframeSpacing is internal: each quality is one rendition, named after its longest GoP.
    const renditions = s.videoSource.getRenditions()
    expect(renditions.map((r) => r.id)).toEqual(renditions.map((r) => r.id.replace(/_[^_]+$/, '_6s')))
    expect(new Set(renditions.map((r) => r.height)).size).toBe(renditions.length)
    const joined = s.requestsOf('video')[0]!.track
    expect(joined).toMatch(/_500ms$/)
    // Up the ladder of the joined quality to the 6 s GoP, which then delivers.
    const longest = joined.replace(/_500ms$/, '_6s')
    await s.until(() => s.objectsOf(longest).length > 0, `objects of ${longest}`)
    expect(s.requestsOf('video').every((r) => r.track.startsWith(joined.replace(/_500ms$/, '')))).toBe(true)
    // A climb is no rendition change.
    expect(s.eventsOf('renditionchange')).toEqual([])
    await s.played(1)
    s.since(playing).expectSmooth()
  })
})
