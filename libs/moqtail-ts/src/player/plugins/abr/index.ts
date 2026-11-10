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
import { DefaultAbrController } from './controller'

export { DefaultAbrController } from './controller'

/**
 * @public
 * An ABR controller for media sources that don't switch themselves. Priority `Infinity`, so any
 * other active ABR plugin wins over it.
 */
export const abr = definePlugin({
  name: 'abr',
  priority: Infinity,
  init: () => new DefaultAbrController(),
  features: (controller) => [{ type: 'abr-controller', item: controller }],
})
