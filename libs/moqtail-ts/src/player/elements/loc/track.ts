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

/** What a catalog track means to the LOC source: decoder config and pad capabilities. */

import { CapabilityCode, type Capabilities } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'
import type { PadProperties } from '@/player/element/pad'

/** A pad carrying a `video` or `audio` LOC set. */
export function locCapabilities(direction: 'in' | 'out', role: string): Capabilities {
  return [
    { direction, code: CapabilityCode.PACKAGING, value: 'loc' },
    { direction, code: CapabilityCode.ROLE, value: role },
  ]
}

/**
 * The WebCodecs config for `track`, decoding `codec`. A video track's `width` and `height` are
 * its maximum encoded size (MSF 5.2.27-28), which WebCodecs takes as a hint.
 */
export function decoderConfigOf(track: CatalogTrack, codec: string): NonNullable<PadProperties['decoderConfig']> {
  if (track.role === 'audio') {
    return { codec, sampleRate: track.samplerate!, numberOfChannels: Number(track.channelConfig) }
  }
  return {
    codec,
    ...(track.width !== undefined && { codedWidth: track.width }),
    ...(track.height !== undefined && { codedHeight: track.height }),
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { track } = await import('@/player/testing/elements')

  test('decoderConfigOf: coded size for video, sample rate and channels for audio', () => {
    const video = { ...track('v1'), packaging: 'loc', role: 'video', codec: 'avc3.64001f', width: 1280, height: 720 }
    expect(decoderConfigOf(video, 'avc3.64001f')).toEqual({ codec: 'avc3.64001f', codedWidth: 1280, codedHeight: 720 })
    const audio = {
      ...track('a1'),
      packaging: 'loc',
      role: 'audio',
      codec: 'opus',
      samplerate: 48_000,
      channelConfig: '2',
    }
    expect(decoderConfigOf(audio, 'opus')).toEqual({ codec: 'opus', sampleRate: 48_000, numberOfChannels: 2 })
  })
}
