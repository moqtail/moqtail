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

import { MOQtailVideoElement } from './element'

export { MOQtailVideoElement } from './element'

/** @public Registers `MOQtailVideoElement` as `name`. A no-op when it's taken or there's no `customElements`. */
export function defineMOQtailVideo(name = 'moqtail-video'): void {
  if (typeof customElements === 'undefined' || customElements.get(name)) return
  customElements.define(name, MOQtailVideoElement)
}
