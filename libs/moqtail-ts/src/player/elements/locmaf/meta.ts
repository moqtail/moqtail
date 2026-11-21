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

import type { Identifiers, SourceElementDefinition } from '@/player/element/define'
import { MEDIA_SOURCE } from '@/player/element/interfaces'
import { cmafCapabilities } from '@/player/elements/cmaf/track'

/** The LOCMAF packaging version this element decodes (draft-einarsson-moq-locmaf-01 §5). */
const LOCMAF_VERSION = '0.3'

/**
 * The LOCMAF source without its `init`, which the built-in loads on first use. Its lower
 * priority prefers it over a CMAF set of the same media, since LOCMAF has less overhead.
 */
export const LOCMAF_META = {
  type: 'source',
  name: 'locmaf',
  priority: 90,
  capabilities: [cmafCapabilities('out', 'video'), cmafCapabilities('out', 'audio')],
  provides: [MEDIA_SOURCE],
  probe: probeLocmaf,
} satisfies Omit<SourceElementDefinition, 'init'>

/**
 * Accepts an unencrypted LOCMAF `video` or `audio` track of a version it decodes: a receiver
 * must not subscribe to another (§5). Encryption is out of scope, as for CMAF.
 */
export function probeLocmaf(identifiers: Identifiers): boolean {
  const { packaging, role } = identifiers
  if (packaging !== 'locmaf' || (role !== 'video' && role !== 'audio')) return false
  if (identifiers['locmafVersion'] !== LOCMAF_VERSION) return false

  return identifiers['encryptionScheme'] === undefined && identifiers['contentProtectionRefIDs'] === undefined
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('probeLocmaf: unencrypted LOCMAF 0.3 video or audio', () => {
    const video = { packaging: 'locmaf', locmafVersion: '0.3', role: 'video', codec: 'avc1.64001f' }
    expect([probeLocmaf(video), probeLocmaf({ ...video, role: 'audio', codec: 'mp4a.40.2' })]).toEqual([true, true])

    for (const other of [
      { ...video, packaging: 'cmaf' },
      { ...video, locmafVersion: '0.2' },
      { ...video, locmafVersion: undefined },
      { ...video, role: 'subtitle' },
      { ...video, contentProtectionRefIDs: ['drm'] },
    ]) {
      expect(probeLocmaf(other)).toBe(false)
    }
  })
}
