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

import { defineElement, type Identifiers } from '@/player/element/define'
import { MEDIA_SOURCE, type MediaSourceOptions } from '@/player/element/interfaces'
import { planSource } from '@/player/element/source/switching_set'
import { CmafSource } from './source'
import { cmafCapabilities } from './track'

/**
 * The built-in media source for a CMAF switching set (CMSF). The player registers it, and
 * elements reach it through `context.probe()`, so it isn't exported.
 */
export const Cmaf = defineElement({
  type: 'source',
  name: 'cmaf',
  capabilities: [cmafCapabilities('out', 'video'), cmafCapabilities('out', 'audio')],
  provides: [MEDIA_SOURCE],
  probe: probeCmaf,
  init: async (context, options?: MediaSourceOptions) =>
    new CmafSource(context, options ?? {}, await planSource(context, options, 'cmaf')),
})

/**
 * 1 for an unencrypted CMAF `video` or `audio` track, else 0. Whether the codec can be
 * decoded is left to the element that decodes it, since a transform may sit in between.
 * The catalog element infers a missing `role` before probing.
 */
export function probeCmaf(identifiers: Identifiers): number {
  const { packaging, role } = identifiers
  if (packaging !== 'cmaf' || (role !== 'video' && role !== 'audio')) return 0
  // Encryption is out of scope
  if (identifiers['encryptionScheme'] !== undefined || identifiers['contentProtectionRefIDs'] !== undefined) return 0
  return 1
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('probeCmaf: unencrypted CMAF video or audio, whatever the codec', () => {
    const video = { packaging: 'cmaf', role: 'video', codec: 'avc1.64001f' }
    for (const track of [
      video,
      { ...video, role: 'audio', codec: 'mp4a.40.2' },
      { ...video, codec: 'hev1.1.6.L93.B0' },
    ]) {
      expect(probeCmaf(track)).toBe(1)
    }
    for (const other of [
      { ...video, packaging: 'loc' },
      { ...video, role: 'subtitle' },
      { packaging: 'cmaf', codec: 'avc1.64001f' },
      { ...video, encryptionScheme: 'cenc' },
      { ...video, contentProtectionRefIDs: ['drm'] },
    ]) {
      expect(probeCmaf(other)).toBe(0)
    }
  })
}
