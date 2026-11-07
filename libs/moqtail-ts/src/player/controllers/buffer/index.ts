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

import type { MediaSinkElement } from '@/player/element/interfaces'
import type { MOQtailPlayer } from '@/player/player'

/**
 * @public
 * Set through `opts.buffer`. The player attaches it once a live stream is ready, and detaches
 * it on `destroy()` and when the stream turns on-demand. Times are in seconds. A controller
 * holds one player's state, so give each player its own instance.
 */
export interface BufferController {
  attach(player: MOQtailPlayer, sink: MediaSinkElement): void
  detach(): void
  /** Seconds kept behind `currentTime`; replaces the sink's own `backBuffer` while attached. */
  backBuffer?(): number
}

export { defaultBufferController } from './default'
