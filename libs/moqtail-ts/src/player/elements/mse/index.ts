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
import { PlayerError } from '@/player/error'
import { checkOptions, MSE_META, type MediaSources } from './meta'
import { MSESink, type MSEOptions } from './sink'

/**
 * @public
 * Plays CMAF video and audio pads on an `HTMLVideoElement`, through `ManagedMediaSource`
 * where it exists and `MediaSource` otherwise. `player.sink(video)` adds it with `{ video }`.
 * Importing it from `moqtail/player/elements/mse` puts its code in your bundle; the `MSE` of
 * `moqtail/player` loads it on first use.
 */
export const MSE = defineElement({
  ...MSE_META,
  init: async (context, options: MSEOptions) => {
    checkOptions(options)

    const { ManagedMediaSource, MediaSource } = globalThis as MediaSources
    const Backend = ManagedMediaSource ?? MediaSource
    if (!Backend) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'Neither ManagedMediaSource nor MediaSource is available', {
        element: 'mse',
      })
    }

    return new MSESink(context, options, Backend, Backend === ManagedMediaSource)
  },
})
