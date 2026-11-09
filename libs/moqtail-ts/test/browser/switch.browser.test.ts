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
 * Manual rendition switches on live content: the default (SUBSCRIBE + REQUEST_UPDATE) and
 * `SWITCH_FROM` (`switchFrom`) requests, fast switching across GoP ladders, and that a switch
 * replaces the old rendition's buffer.
 *
 * With fast switching, a switch enters at the closest switch point, interrupting a climb that
 * waits for a later one; on `ch07_all` every switch lands within 0.9 s. Once the new track
 * plays, nothing of the old one is left buffered behind it.
 */

import { ch07, ch07_all, describe, expect, test, type Session, warmUp } from './harness/test'

/** Switches `count` times, each time to the next rendition in catalog order. */
async function cycle(s: Session, count: number) {
  for (let i = 0; i < count; i++) {
    await s.switchTo((renditions) => {
      const active = renditions.findIndex((r) => r.active)
      return renditions[(active + 1) % renditions.length]
    })
    await s.played(1)
    // Once the new track plays, the old one's data before it is gone: a seek back can't show it.
    const switched = s.switches.at(-1)!
    expect(s.buffered('video')[0]![0], `video buffered after ${switched.to}`).toBeGreaterThan(switched.playhead - 0.2)
  }
}

/** For the network: from `switch()` to the first packet, past the wait for the switch point. */
const MARGIN_MS = 400

/**
 * Every switch landed within the shortest keyframe interval of its target's quality, plus
 * `MARGIN_MS`: the closest switch point is never further away, and a climb waiting for a
 * later one doesn't hold it up. On `ch07_all` that's 0.9 s, on `ch07` 2.4 s.
 */
function expectAtClosestSwitchPoint(s: Session) {
  const height = (track: string) => s.catalog!.tracks.find((t) => t.name === track)!.height
  const interval = (track: string) =>
    Math.min(...s.catalog!.tracks.filter((t) => t.height === height(track)).map((t) => s.keyframeInterval(t.name)))
  const late = s.switches.filter((w) => w.ms > interval(w.to) * 1000 + MARGIN_MS)
  expect(late.map((w) => `${w.from} > ${w.entered} ${w.ms.toFixed(0)} ms`)).toEqual([])
}

describe('switch', () => {
  warmUp(ch07, ch07_all)

  test.for([
    ['ch07', false],
    ['ch07', true],
    ['ch07_all', false],
    ['ch07_all', true],
  ] as const)(
    '%s switchFrom=%s: switches land at the next switch point and replace the buffer',
    async ([name, switchFrom], { open }) => {
      const s = await open((name === 'ch07' ? ch07 : ch07_all).uri(), { switchFrom })
      const playing = s.mark()
      await cycle(s, 2)
      expectAtClosestSwitchPoint(s)
      if (switchFrom)
        expect(
          s
            .requestsOf('video')
            .slice(1)
            .map((r) => r.kind),
        ).toContain('switch')
      s.since(playing).expectSmooth()
    },
  )
})
