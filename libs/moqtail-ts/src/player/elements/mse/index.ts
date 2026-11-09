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

import { defineElement } from '@/player/element/define'
import { MEDIA_SINK } from '@/player/element/interfaces'
import { cmafCapabilities } from '@/player/elements/cmaf/track'
import { PlayerError } from '@/player/error'
import { MSESink, type MSEOptions } from './sink'

/**
 * @public
 * Plays CMAF video and audio pads on an `HTMLVideoElement`, through `ManagedMediaSource`
 * where it exists and `MediaSource` otherwise. `player.sink(video)` adds it with `{ video }`.
 */
export const MSE = defineElement({
  type: 'sink',
  name: 'mse',
  capabilities: [cmafCapabilities('in', 'video'), cmafCapabilities('in', 'audio')],
  provides: [MEDIA_SINK],
  init: async (context, options: MSEOptions) => {
    checkOptions(options)
    const { ManagedMediaSource, MediaSource } = globalThis as {
      ManagedMediaSource?: typeof globalThis.MediaSource
      MediaSource?: typeof globalThis.MediaSource
    }
    const Backend = ManagedMediaSource ?? MediaSource
    if (!Backend) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'Neither ManagedMediaSource nor MediaSource is available', {
        element: 'mse',
      })
    }
    return new MSESink(context, options, Backend, Backend === ManagedMediaSource)
  },
})

/** Options come from the app, so bad values are programming errors. */
export function checkOptions(options: MSEOptions): void {
  for (const key of ['backBuffer', 'forwardCap', 'liveEdgeDelay'] as const) {
    const value = options[key]
    const valid = value === undefined || (Number.isFinite(value) && (key === 'forwardCap' ? value > 0 : value >= 0))
    if (!valid)
      throw new TypeError(`MSE option ${key} must be a finite number ${key === 'forwardCap' ? '> 0' : '>= 0'}`)
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const video = {} as HTMLVideoElement

  test('checkOptions rejects values the sink cannot work with', () => {
    expect(() => checkOptions({ video, backBuffer: 0, forwardCap: 1, liveEdgeDelay: 0 })).not.toThrow()
    for (const bad of [
      { forwardCap: 0 },
      { backBuffer: -1 },
      { liveEdgeDelay: Number.NaN },
      { backBuffer: Infinity },
    ]) {
      expect(() => checkOptions({ video, ...bad })).toThrow(TypeError)
    }
  })
}
