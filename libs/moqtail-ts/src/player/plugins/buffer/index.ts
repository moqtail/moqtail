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
import { CATCHUP_RATE, LiveBufferController, TARGET_LATENCY } from './live'

/** @public */
export interface LiveBufferConfig {
  /** The playback rate when behind the target latency; `2 - catchupRate` when ahead. Between 1 and 2, default 1.1. */
  catchupRate?: number
  /**
   * Seconds of latency to hold when the catalog sets no `targetLatency` or `buffers`. Measured like the
   * catalog's: wallclock latency when the timeline has it, else the forward buffer. Above 0, default 1.
   */
  targetLatency?: number
}

/**
 * @public
 * A buffer controller that holds the catalog's target latency, else `targetLatency` (1 s), through the playback rate.
 * Priority `Infinity`, so any other active buffer plugin wins over it.
 */
export const liveBuffer = definePlugin({
  name: 'live-buffer',
  priority: Infinity,
  init: ({ catchupRate = CATCHUP_RATE, targetLatency = TARGET_LATENCY }: LiveBufferConfig = {}) => {
    if (!(catchupRate > 1 && catchupRate < 2)) throw new TypeError('catchupRate must be between 1 and 2')
    if (!(targetLatency > 0 && targetLatency < Infinity)) throw new TypeError('targetLatency must be a positive number')
    return new LiveBufferController(catchupRate, targetLatency)
  },
  features: (controller) => [{ type: 'buffer-controller', item: controller }],
})
