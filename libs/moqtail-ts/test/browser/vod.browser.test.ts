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
 * VOD: location and media time ranges from the URI, seeking, and playing to the end.
 *
 * A switch at mid-point fetches the new quality from the playhead and clears the old one's
 * buffer. A seek back to the start, or to any region not fetched since, requests it from
 * upstream through the source.
 */

import { ch07_vod, describe, expect, test } from './harness/test'

// A small forward buffer, so a seek a few groups ahead is out of the buffer.
const sink = { forwardCap: 4 }

/** `from` to `to`, inclusive. */
const span = (from: bigint, to: bigint) => Array.from({ length: Number(to - from) + 1 }, (_, i) => from + BigInt(i))
/** `[start, end]` pairs to 0.1 s. */
const tenths = (ranges: [number, number][]) => ranges.map((r) => r.map((t) => Math.round(t * 10) / 10))
/** Every inner range lies inside one of the outer ones. */
const covered = (outer: [number, number][], inner: [number, number][]) =>
  inner.every(([start, end]) => outer.some(([s, e]) => s <= start && end <= e))

describe('vod', () => {
  test('plays from the start to the end, every group once, with no buffer controller attached', async ({ open }) => {
    let attached = 0
    const buffer = { attach: () => void attached++, detach() {} }
    const s = await open(ch07_vod.uri(), { playbackRate: 16, player: { buffer } })
    expect(s.firstGroup('video')).toBe(0n)
    await s.ended(60_000)
    expect(s.stateTrace()).toMatch(/^connecting > ready > playing( > stalled > playing)* > ended$/)
    const groups = s.groups('video')
    expect(groups).toEqual(span(0n, groups.at(-1)!))
    expect(s.video.currentTime).toBeCloseTo(s.video.duration, 0)
    expect(attached).toBe(0)
  })

  test('a mid-point switch replaces the whole buffer from the playhead; a seek back fetches the start again', async ({
    open,
  }) => {
    // The default 60 s forward cap fills the buffer far ahead with the old rendition.
    const s = await open(ch07_vod.uri())
    await s.played(3)
    const switching = s.mark()
    const w = await s.switchTo((renditions) => renditions.find((r) => !r.active && r.height === 540))
    const after = s.since(switching).newRequests()
    // From the group holding the playhead, on the new track only.
    expect(after.requests.map((r) => `${r.kind} ${r.track}`)).toEqual([`fetch ${w.to}`])
    expect(after.groups('video')[0]).toBe(s.groupAt(w.playhead))
    // Nothing of the old rendition is left, behind the playhead or ahead of it: one range, from that group.
    const video = s.buffered('video')
    expect(video).toHaveLength(1)
    expect(video[0]![0]).toBeGreaterThan(w.playhead - ch07_vod.gop! - 0.1)
    // The playhead stayed put, give or take a jump onto the new data.
    expect(s.video.currentTime).toBeGreaterThanOrEqual(w.playhead)
    expect(s.video.currentTime).toBeLessThan(w.playhead + w.shownMs / 1000 + 0.5)

    const back = s.mark()
    await s.seek(0)
    await s.played(1)
    const refetched = s.since(back).newRequests()
    expect(refetched.requestsOf('video').map((r) => `${r.kind} ${r.track}`)).toEqual([`fetch ${w.to}`])
    expect(refetched.groups('video')[0]).toBe(0n)
    expect(s.video.videoHeight).toBe(540)
  })

  test('a switch after the whole range was delivered still fetches the new quality from the playhead', async ({
    open,
  }) => {
    // Groups 2 to 6, 4 s to 14 s: all of it fits the forward buffer, so the source writes its eos early.
    const s = await open(ch07_vod.uri('location-range=2-6'))
    await s.until(() => (s.buffered('video').at(-1)?.[1] ?? 0) > 13.9, 'the whole range buffered')
    await s.played(0.5)
    const switching = s.mark()
    const w = await s.switchTo((renditions) => renditions.find((r) => !r.active && r.height === 540))
    const after = s.since(switching).newRequests()
    expect(after.requestsOf('video').map((r) => `${r.kind} ${r.track}`)).toEqual([`fetch ${w.to}`])
    expect(after.groups('video')[0]).toBe(s.groupAt(w.playhead))
  })

  test('a location range fetches exactly its video groups, and audio for the same media time', async ({ open }) => {
    const s = await open(ch07_vod.uri('location-range=2-4'), { playbackRate: 4 })
    await s.ended()
    s.expectGroups('video', [2, 3, 4])
    // Audio groups are about 0.7 s, so the range maps through the templates: 4 s up to 10 s.
    s.expectGroups('audio', span(s.groupAt(4, 'audio'), s.groupAt(9.999, 'audio')))
    // Combined `buffered` reaches the longest track once ended (MSE), so each is checked on its own.
    expect(tenths(s.buffered('video'))).toEqual([[4, 10]])
    expect(covered(s.buffered('audio'), s.buffered('video'))).toBe(true)
    expect(s.requestsOf('video').map((r) => r.kind)).toEqual(['fetch'])
  })

  test('two location ranges fetch both and play through the hole between them', async ({ open }) => {
    const s = await open(ch07_vod.uri('location-range=2-4', 'location-range=10-12'), { playbackRate: 4 })
    await s.ended()
    s.expectGroups('video', [2, 3, 4, 10, 11, 12])
    expect(tenths(s.buffered('video'))).toEqual([
      [4, 10],
      [20, 26],
    ])
    expect(covered(s.buffered('audio'), s.buffered('video'))).toBe(true)
    expect(s.video.currentTime).toBeGreaterThan(25)
  })

  test('a media time range fetches the groups holding it', async ({ open }) => {
    const s = await open(ch07_vod.uri('mediatime-range=4000-9999'), { playbackRate: 4 })
    await s.ended()
    s.expectGroups('video', [2, 3, 4])
    s.expectGroups('audio', span(s.groupAt(4, 'audio'), s.groupAt(9.999, 'audio')))
  })

  test('a seek inside the buffer requests nothing; one out of it fetches its group and plays on', async ({ open }) => {
    const s = await open(ch07_vod.uri(), { sink })
    await s.played(1)
    const inside = s.mark()
    await s.seek(0.2)
    await s.played(0.5)
    expect(s.since(inside).requests).toEqual([])
    s.since(inside).expectSmooth()

    const target = 40
    const outside = s.mark()
    await s.seek(target)
    await s.played(1)
    const after = s.since(outside).newRequests()
    // Only from the group holding the target: nothing before it is fetched again.
    expect(after.groups('video')[0]).toBe(s.groupAt(target))
    expect(after.requestsOf('video').map((r) => r.kind)).toEqual(['fetch'])
    expect(s.player.state).toBe('playing')
    const resumed = s.mark()
    await s.played(2)
    s.since(resumed).expectSmooth()
  })
})
