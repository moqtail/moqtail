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
import { AudioConfig } from '@/model/property/audio_config'
import { CodecString } from '@/model/property/codec_string'
import { findProperty } from '@/model/property/find'
import { Timescale } from '@/model/property/timescale'
import { Timestamp } from '@/model/property/timestamp'
import type { TrackProperty } from '@/model/property/track_property'
import { VideoConfig } from '@/model/property/video_config'
import { CapabilityCode, type Capabilities, type Identifiers } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'
import type { PadProperties } from '@/player/element/pad'

/** The AAC sampling frequency indexes (ISO/IEC 14496-3 1.6.3.4). */
export const AAC_SAMPLE_RATES = [
  96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000, 7_350,
]
export const EXPLICIT_SAMPLE_RATE = 0xf
// Channels per AAC channel configuration, where 0 defers to a program config element
const AAC_CHANNELS = [undefined, 1, 2, 3, 4, 5, 6, 8]

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

/**
 * The track of a URL played without a catalog, from its properties (LOC-05): the role and init
 * data from VIDEO_CONFIG or AUDIO_CONFIG, in the track properties first, then the first object
 * with one, and the codec from CODEC_STRING. Undefined when they don't tell, or for audio whose format it can't read.
 */
export function trackOfProperties(identifiers: Identifiers): CatalogTrack | undefined {
  const { track: name, trackBytes, namespace, isLive, trackProperties, objects = [] } = identifiers
  if (name === undefined || !trackBytes || !namespace || isLive === undefined) return undefined

  const objectProperties = objects.map((o) => o.properties)
  const config =
    configOf(trackProperties, 'track-property') ??
    objectProperties.map((p) => configOf(p, 'object-property')).find((c) => c !== undefined)

  const codec = [trackProperties, ...objectProperties].map((p) => findProperty(p, CodecString)).find((c) => c)?.codec
  if (!config || !codec) return undefined

  const { role, type, property, data } = config

  // AudioDecoder.configure() needs the sample rate and channels before any AudioData exists
  const format = role === 'audio' ? audioFormatOf(codec, data) : undefined
  if (role === 'audio' && !format) return undefined

  return {
    name,
    nameBytes: trackBytes,
    namespace,
    packaging: 'loc',
    role,
    isLive,
    codec,
    init: { type, property },
    ...format,
  }
}

/** The sample rate and channel count of an AAC AudioSpecificConfig or an OpusHead; undefined for other codecs. */
export function audioFormatOf(
  codec: string,
  config: Uint8Array,
): { samplerate: number; channelConfig: string } | undefined {
  if (codec === 'opus') return opusFormat(config)
  if (codec.startsWith('mp4a.40.')) return aacFormat(config)
  return undefined
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

function configOf(
  properties: readonly (KeyValuePair | TrackProperty)[] | null | undefined,
  type: 'track-property' | 'object-property',
) {
  const video = findProperty(properties, VideoConfig)
  if (video) return { role: 'video', type, property: VideoConfig.TYPE, data: video.config }
  const audio = findProperty(properties, AudioConfig)
  if (audio) return { role: 'audio', type, property: AudioConfig.TYPE, data: audio.config }
  return undefined
}

/** RFC 7845 5.1: the channel count is byte 9, and Opus decodes at 48 kHz whatever the input rate. */
function opusFormat(head: Uint8Array): { samplerate: number; channelConfig: string } | undefined {
  const magic = new TextDecoder().decode(head.subarray(0, 8))
  if (magic !== 'OpusHead' || !head[9]) return undefined
  return { samplerate: 48_000, channelConfig: String(head[9]) }
}

/** ISO/IEC 14496-3 1.6.2.1: object type, sampling frequency index or explicit rate, channel configuration. */
function aacFormat(config: Uint8Array): { samplerate: number; channelConfig: string } | undefined {
  let offset = 0
  const read = (bits: number) => {
    let value = 0
    for (const end = offset + bits; offset < end; offset++) {
      value = value * 2 + (((config[offset >> 3] ?? 0) >> (7 - (offset & 7))) & 1)
    }
    return value
  }

  if (read(5) === 31) read(6) // escaped object type

  const index = read(4)
  const samplerate = index === EXPLICIT_SAMPLE_RATE ? read(24) : AAC_SAMPLE_RATES[index]
  const channels = AAC_CHANNELS[read(4)]

  if (offset > config.length * 8 || !samplerate || !channels) return undefined

  return { samplerate, channelConfig: String(channels) }
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

  test('audioFormatOf: AAC AudioSpecificConfig and OpusHead, nothing for other codecs', () => {
    // testsrc's AAC-LC at 48 kHz, stereo, with an SBR sync extension
    const aac = new Uint8Array([0x11, 0x90, 0x56, 0xe5, 0x00])
    expect(audioFormatOf('mp4a.40.2', aac)).toEqual({ samplerate: 48_000, channelConfig: '2' })

    const head = new Uint8Array(19)
    head.set(new TextEncoder().encode('OpusHead'))
    head[9] = 1

    expect(audioFormatOf('opus', head)).toEqual({ samplerate: 48_000, channelConfig: '1' })
    expect(audioFormatOf('flac', aac)).toBeUndefined()
    expect(audioFormatOf('mp4a.40.2', new Uint8Array([0x11]))).toBeUndefined()
  })
}
