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

import { lazyElement, type SourceElementDefinition, type TransformElementDefinition } from '@/player/element/define'
import { CatalogSource } from './catalog'
import { Cmaf } from './cmaf'
import { Loc } from './loc'
import { LOCMAF_META } from './locmaf/meta'
import { MediaTimeline } from './mediatimeline'
import { MSE_META } from './mse/meta'
import { PLAYOUT_META } from './playout/meta'
import { SapTimeline } from './saptimeline'
import { WEBCODECS_META } from './webcodecs/meta'

// Lazy: each serves one route (MSE or Playout), so an app fetches only its own. The
// others are small or needed by every stream, so a lazy load would only add a fetch.
const Locmaf = lazyElement(LOCMAF_META, () => import('./locmaf').then((m) => m.Locmaf))
const WebCodecs = lazyElement(WEBCODECS_META, () => import('./webcodecs').then((m) => m.WebCodecs))

/**
 * @public
 * Plays CMAF video and audio pads on an `HTMLVideoElement`, and loads its code on first use.
 * `player.sink(video)` adds it with `{ video }`. The `MSE` of `moqtail/player/elements/mse`
 * is the same element, bundled with your app.
 */
export const MSE = lazyElement(MSE_META, () => import('./mse').then((m) => m.MSE))

/**
 * @public
 * Plays live `raw` video and audio pads on a canvas, and loads its code on first use.
 * `player.sink(canvas, options)` adds it with `{ ...options, canvas }`. The `Playout` of
 * `moqtail/player/elements/playout` is the same element, bundled with your app.
 */
export const Playout = lazyElement(PLAYOUT_META, () => import('./playout').then((m) => m.Playout))

/**
 * The source elements every player registers after its own and its extensions'. They're
 * reached through the probe only, so they aren't exported from `moqtail/player`.
 */
export const BUILT_IN_SOURCES: readonly SourceElementDefinition[] = [
  CatalogSource,
  Cmaf,
  Locmaf,
  Loc,
  MediaTimeline,
  SapTimeline,
]

/**
 * The transforms the runtime may load to connect a pad no declared element takes, tried after
 * the player's own and its extensions'. Like the sources, they're reached through the runtime
 * only, so they aren't exported from `moqtail/player`.
 */
export const BUILT_IN_TRANSFORMS: readonly TransformElementDefinition[] = [WebCodecs]
