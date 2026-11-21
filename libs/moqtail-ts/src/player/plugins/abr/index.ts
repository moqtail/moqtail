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
import { MlAbrController, TICK_MS, type Decision } from './controller'
import { BUNDLED_MODEL } from './model'
import { Policy, type ModelBlob } from './policy'

export { MlAbrController, type Decision } from './controller'
export { Action, type Observation } from './features'
export type { ModelBlob } from './policy'

/** @public */
export interface MlAbrConfig {
  /** Milliseconds between decisions. Above 0, default 500, which the bundled model was trained at. */
  tickMs?: number
  /** A model to use instead of the bundled one. */
  model?: ModelBlob
  /** Called with every decision, for logging. */
  onDecision?: (decision: Decision) => void
}

/** Decoded once, then shared by every player. */
let bundled: Promise<Policy> | undefined

/**
 * @public
 * An ABR controller driven by a small neural network, for live streams. Priority `Infinity`, so any
 * other active ABR plugin wins over it.
 */
export const abr = definePlugin({
  name: 'abr',
  priority: Infinity,
  init: ({ tickMs = TICK_MS, model, onDecision }: MlAbrConfig = {}) => {
    if (!(tickMs > 0 && tickMs < Infinity)) throw new TypeError('tickMs must be a positive number')

    const policy = model ? Policy.load(model) : (bundled ??= Policy.load(BUNDLED_MODEL))
    return new MlAbrController(tickMs, policy, onDecision)
  },
  features: (controller) => [{ type: 'abr-controller', item: controller }],
})
