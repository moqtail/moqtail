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
import type { AbrController } from '@/player/plugin'
import type { MOQtailPlayer } from '@/player/player'

/** @public The `abr()` controller. For now it keeps the current quality: it never switches. */
export class DefaultAbrController implements AbrController {
  attach(_player: MOQtailPlayer, _sink: MediaSinkElement): void {}

  detach(): void {}
}
