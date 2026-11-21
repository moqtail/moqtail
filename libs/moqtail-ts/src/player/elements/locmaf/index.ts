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
import type { MediaSourceOptions } from '@/player/element/interfaces'
import { planSource } from '@/player/element/source/switching_set'
import { LOCMAF_META } from './meta'
import { LocmafSource } from './source'

/**
 * @public
 * The media source for a LOCMAF switching set. It rebuilds CMAF chunks, so its pad carries CMAF.
 * The player loads it on first use; importing it from `moqtail/player/elements/locmaf` and passing
 * it to `registerElement()` puts its code in your bundle.
 */
export const Locmaf = defineElement({
  ...LOCMAF_META,
  init: async (context, options: MediaSourceOptions = {}) => {
    const plan = await planSource(context, options, 'locmaf')
    return new LocmafSource(context, options, plan)
  },
})
