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
import { LocSource } from './source'
import { locCapabilities } from './track'

/**
 * The built-in media source for a LOC switching set. The player registers it, and
 * elements reach it through `context.probe()`, so it isn't exported.
 */
export const Loc = defineElement({
  type: 'source',
  name: 'loc',
  capabilities: [locCapabilities('out', 'video'), locCapabilities('out', 'audio')],
  provides: [MEDIA_SOURCE],
  probe: probeLoc,
  init: async (context, options?: MediaSourceOptions) =>
    new LocSource(context, options ?? {}, await planSource(context, options, 'loc')),
})

/**
 * Accepts an unencrypted LOC `video` or `audio` track. Whether the codec can be decoded is
 * left to the element that decodes it, since a transform may sit in between. Audio needs a
 * channel count for WebCodecs, so its `channelConfig` must be a positive integer, which
 * MSF 5.2.30 doesn't require.
 */
export function probeLoc(identifiers: Identifiers): boolean {
  const { packaging, role, channelConfig } = identifiers
  if (packaging !== 'loc' || (role !== 'video' && role !== 'audio')) return false
  if (role === 'audio' && !/^[1-9]\d*$/.test(channelConfig ?? '')) return false
  // Encryption is out of scope
  return identifiers['encryptionScheme'] === undefined && identifiers['contentProtectionRefIDs'] === undefined
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('probeLoc: unencrypted LOC video, or audio with a channel count, whatever the codec', () => {
    const video = { packaging: 'loc', role: 'video', codec: 'avc3.64001f' }
    const audio = { packaging: 'loc', role: 'audio', codec: 'opus', samplerate: 48_000, channelConfig: '2' }
    for (const track of [video, audio, { ...video, codec: 'av01.0.08M.08' }, { ...audio, codec: 'mp4a.40.2' }]) {
      expect(probeLoc(track)).toBe(true)
    }
    for (const other of [
      { ...video, packaging: 'cmaf' },
      { ...video, role: 'subtitle' },
      { packaging: 'loc', codec: 'avc3.64001f' },
      { ...video, encryptionScheme: 'cenc' },
      { ...video, contentProtectionRefIDs: ['drm'] },
      { ...audio, channelConfig: '5.1' },
      { ...audio, channelConfig: 'mono' },
    ]) {
      expect(probeLoc(other)).toBe(false)
    }
  })
}
