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

/** What a catalog track and its objects mean to LOC elements: decoder config, pad capabilities, timestamps. */

import type { KeyValuePair } from '@/model/common/pair'
import { findProperty } from '@/model/property/find'
import { Timescale } from '@/model/property/timescale'
import { Timestamp } from '@/model/property/timestamp'
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
export function decoderConfigOf(
  track: Partial<CatalogTrack>,
  codec: string,
): NonNullable<PadProperties['decoderConfig']> {
  if (track.role === 'audio') {
    return { codec, sampleRate: track.samplerate!, numberOfChannels: Number(track.channelConfig) }
  }
  return {
    codec,
    ...(track.width !== undefined && { codedWidth: track.width }),
    ...(track.height !== undefined && { codedHeight: track.height }),
  }
}

/** An object's LOC Timestamp in µs, read with its own TIMESCALE, else the track's `timescale` (LOC Table 1). */
export function timestampMicros(
  properties: readonly KeyValuePair[],
  timescale: number | undefined,
): number | undefined {
  const timestamp = findProperty(properties, Timestamp)
  if (!timestamp) return undefined
  const own = findProperty(properties, Timescale)?.timescale
  return timestamp.toMicros(own ?? (timescale === undefined ? undefined : BigInt(timescale)))
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
