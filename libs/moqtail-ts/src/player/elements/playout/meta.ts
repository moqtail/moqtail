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

import { CapabilityCode, type Capabilities, type SinkElementDefinition } from '@/player/element/define'
import { MEDIA_SINK } from '@/player/element/interfaces'

/** The playout sink without its `init`, which the built-in loads on first use. */
export const PLAYOUT_META = {
  type: 'sink',
  name: 'playout',
  capabilities: [rawCapabilities('in', 'video'), rawCapabilities('in', 'audio')],
  provides: [MEDIA_SINK],
  threads: ({ worker }) =>
    ['element:playout', 'OffscreenCanvas', 'requestAnimationFrame'].every((name) => worker?.has(name))
      ? 'both'
      : 'main',
} satisfies Omit<SinkElementDefinition, 'init'>

function rawCapabilities(direction: 'in' | 'out', role: string): Capabilities {
  return [
    { direction, code: CapabilityCode.PACKAGING, value: 'raw' },
    { direction, code: CapabilityCode.ROLE, value: role },
  ]
}
