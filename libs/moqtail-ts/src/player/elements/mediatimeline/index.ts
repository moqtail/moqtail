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
import { MediaTimelineElement } from './timeline'

/** The built-in source for media timeline tracks (MSF 7). Loaded by the catalog, it gets no pad. */
export const MediaTimeline = defineElement({
  type: 'source',
  name: 'mediatimeline',
  capabilities: [],
  probe: (ids) => ids.packaging === 'mediatimeline',
  init: async (context) => {
    const timeline = new MediaTimelineElement(context)
    await timeline.start()
    return timeline
  },
})
