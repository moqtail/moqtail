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
 * Buffer controllers on live `ch07`. Its catalog, re-sent often, sets no latency targets and
 * its live tracks have no template, so `withTargets` gives the parsed tracks some and latency is
 * the forward buffer.
 */

import type { BufferController } from '@/player'
import { CatalogElement } from '@/player/elements/catalog/element'
import { LiveBufferController } from '@/player/plugins/buffer/live'
import { bufferPlugin } from '@/player/testing/elements'
import { ch07, describe, expect, test, warmUp } from './harness/test'

/** Sets `buffers` on every catalog track, before `inner` reads them: on attach and on every catalog update. */
function withTargets(buffers: { target?: number; min?: number; max?: number }, inner: BufferController) {
  return {
    attach: (...args: Parameters<BufferController['attach']>) => {
      const set = (catalog: CatalogElement) => catalog.tracks.forEach((track) => Object.assign(track, { buffers }))
      const [player] = args
      set(player.sources.find((s) => s instanceof CatalogElement) as CatalogElement)
      player.on('catalog', set)
      inner.attach(...args)
    },
    detach: () => inner.detach(),
  }
}

describe('buffer controller', () => {
  warmUp(ch07)

  test('liveBuffer converges to the target latency and returns to rate 1', async ({ open }) => {
    // A strong rate, so the forward buffer grows from ~0.4 s to the target in seconds
    const buffer = withTargets({ target: 1500 }, new LiveBufferController(1.5))
    const s = await open(ch07.uri(), { player: { plugins: [bufferPlugin(buffer)] } })
    await s.until(() => Math.abs(s.latency() - 1.5) < 0.15 && s.video.playbackRate === 1, 'latency at 1.5 s')
    const settled = s.mark()
    await s.played(3)
    expect(Math.abs(s.latency() - 1.5)).toBeLessThan(0.25)
    s.since(settled).expectSmooth()
  })

  test("the attached controller's backBuffer replaces the sink's", async ({ open }) => {
    const buffer: BufferController = { attach() {}, detach() {}, backBuffer: () => 1 }
    const s = await open(ch07.uri(), { player: { plugins: [bufferPlugin(buffer)] } })
    const start = s.buffered()[0]![0]
    await s.played(6)
    // Trimmed to 1 s behind, but never into the playing group
    expect(s.buffered()[0]![0]).toBeGreaterThan(start + 2)
    expect(s.video.currentTime - s.buffered()[0]![0]).toBeLessThan(1 + ch07.gop!)
  })
})
