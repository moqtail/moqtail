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

import { definePlugin } from '@/player/plugin'
import { LIVE_BUFFER_DEFAULTS, LiveBufferController, type LiveBufferConfig } from './live'

export type { LiveBufferConfig } from './live'

/**
 * @public
 * A buffer controller that holds the catalog's target latency, else `targetLatency` (1 s), through the playback rate,
 * or through jumps on a sink whose rate isn't precise, as WebKit's media element, where every rate change stalls playback.
 * Priority `Infinity`, so any other active buffer plugin wins over it.
 */
export const liveBuffer = definePlugin({
  name: 'live-buffer',
  priority: Infinity,
  init: (options: LiveBufferConfig = {}) => {
    const {
      catchupRate = LIVE_BUFFER_DEFAULTS.catchupRate,
      targetLatency = LIVE_BUFFER_DEFAULTS.targetLatency,
      tolerance = LIVE_BUFFER_DEFAULTS.tolerance,
      stallThreshold = LIVE_BUFFER_DEFAULTS.stallThreshold,
      minRate = LIVE_BUFFER_DEFAULTS.minRate,
      jumpFactor = LIVE_BUFFER_DEFAULTS.jumpFactor,
      jumpTolerance = LIVE_BUFFER_DEFAULTS.jumpTolerance,
    } = options
    if (!(catchupRate > 1 && catchupRate < 2)) throw new TypeError('catchupRate must be between 1 and 2')
    if (!(targetLatency > 0 && targetLatency < Infinity)) throw new TypeError('targetLatency must be a positive number')
    if (!(tolerance >= 0 && tolerance < Infinity)) throw new TypeError('tolerance must be a non-negative number')
    if (!(stallThreshold >= 0 && stallThreshold < Infinity)) {
      throw new TypeError('stallThreshold must be a non-negative number')
    }
    if (!(minRate > 0 && minRate <= 1)) throw new TypeError('minRate must be between 0 and 1')
    if (!(jumpFactor > 1 && jumpFactor < Infinity)) throw new TypeError('jumpFactor must be above 1')
    if (!(jumpTolerance > 0 && jumpTolerance < Infinity)) throw new TypeError('jumpTolerance must be a positive number')
    return new LiveBufferController({
      catchupRate,
      targetLatency,
      tolerance,
      stallThreshold,
      minRate,
      jumpFactor,
      jumpTolerance,
    })
  },
  features: (controller) => [{ type: 'buffer-controller', item: controller }],
})
