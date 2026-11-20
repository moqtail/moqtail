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

import { CapabilityCode, defineElement, type Capabilities } from '@/player/element/define'
import { MEDIA_SINK } from '@/player/element/interfaces'
import { PlayerError } from '@/player/error'
import { PlayoutSink, type PlayoutOptions } from './sink'

/**
 * @public
 * Plays live `raw` video and audio pads: video on a canvas, in sync with the audio when the
 * tracks share the epoch clock, audio through an AudioWorklet. It refuses non-live streams and
 * seeks forward only. `player.sink(canvas)` adds it with `{ canvas }`.
 */
export const Playout = defineElement({
  type: 'sink',
  name: 'playout',
  capabilities: [rawCapabilities('in', 'video'), rawCapabilities('in', 'audio')],
  provides: [MEDIA_SINK],
  init: async (context, options: PlayoutOptions) => {
    // Options come from the app, so a bad canvas is a programming error
    if (typeof HTMLCanvasElement === 'undefined' || !(options.canvas instanceof HTMLCanvasElement))
      throw new TypeError('Playout option canvas must be an HTMLCanvasElement')
    if (typeof AudioContext === 'undefined' || typeof requestAnimationFrame === 'undefined') {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'Playout needs AudioContext and requestAnimationFrame', {
        element: 'playout',
      })
    }
    const canvas = options.canvas.getContext('2d')
    if (!canvas) throw new TypeError('The playout canvas already has a context other than "2d"')
    return new PlayoutSink(context, canvas)
  },
})

function rawCapabilities(direction: 'in' | 'out', role: string): Capabilities {
  return [
    { direction, code: CapabilityCode.PACKAGING, value: 'raw' },
    { direction, code: CapabilityCode.ROLE, value: role },
  ]
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('init rejects anything but a canvas', async () => {
    const { detachedContext } = await import('@/player/testing/elements')
    const canvas = {} as HTMLCanvasElement
    await expect(Playout.init(detachedContext(), { canvas })).rejects.toThrow(TypeError)
  })
}
