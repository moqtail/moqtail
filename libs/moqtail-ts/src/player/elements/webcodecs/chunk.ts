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

import { findProperty } from '@/model/property/find'
import { VideoFrameMarking } from '@/model/property/video_frame_marking'
import type { PacketBase } from '@/player/element/pad'
import { AAC_SAMPLE_RATES, EXPLICIT_SAMPLE_RATE, timestampMicros } from '@/player/elements/loc/track'

const START_CODE = [0, 0, 0, 1]
const SHORT_START_CODE = [0, 0, 1]

/**
 * `payload` with start codes in place of 4-byte NAL unit lengths, since the WebCodecs AVC and
 * HEVC registrations read Annex B without a description. Undefined when a length overruns it.
 */
export function annexB(payload: Uint8Array): Uint8Array | undefined {
  // LOC 2.1.3: a length of 1 is a start code
  if (startsWith(payload, START_CODE)) return payload
  // A length of 256 to 511 starts like a 3-byte start code (LOC 2.1.4), so lengths are tried first
  return withStartCodes(payload) ?? (startsWith(payload, SHORT_START_CODE) ? payload : undefined)
}

/**
 * The AudioSpecificConfig (ISO/IEC 14496-3 1.6.2.1) of an `mp4a.40.<object type>` codec, else
 * undefined, also for a channel count its 4-bit channel configuration can't hold.
 */
export function aacDescription(codec: string, sampleRate: number, numberOfChannels: number): Uint8Array | undefined {
  const objectType = Number(/^mp4a\.40\.(\d+)$/.exec(codec)?.[1])
  // Object types from 31 on need an escape this doesn't write
  if (!(objectType > 0 && objectType < 31)) return undefined
  if (!(numberOfChannels > 0 && numberOfChannels < 16)) return undefined

  const index = AAC_SAMPLE_RATES.indexOf(sampleRate)
  const rate: [value: number, bits: number][] =
    index === -1
      ? [
          [EXPLICIT_SAMPLE_RATE, 4],
          [sampleRate, 24],
        ]
      : [[index, 4]]

  const fields: [value: number, bits: number][] = [
    [objectType, 5],
    ...rate,
    // shortcut: the channel count is taken as the channel configuration, wrong for 7.1 (configuration 7), map it once such a track exists
    [numberOfChannels, 4],
    // GASpecificConfig: frameLengthFlag, dependsOnCoreCoder, extensionFlag
    [0, 3],
  ]

  const bits = fields.map(([value, size]) => value.toString(2).padStart(size, '0')).join('')
  return Uint8Array.from(bits.match(/.{8}/g)!, (byte) => parseInt(byte, 2))
}

export function chunkType(packet: PacketBase, kind: 'video' | 'audio'): EncodedVideoChunkType {
  const marking = findProperty(packet.properties, VideoFrameMarking)
  // LOC 4.2: a group starts with a key frame
  const key = kind === 'audio' || (marking ? marking.independent : packet.location.object === 0n)
  return key ? 'key' : 'delta'
}

/** The µs timestamp of an object of a track with `timescale`. Call it on arrival: an object without one gets the time of the call. */
export function chunkTimestamp(packet: PacketBase, timescale: number | undefined): number {
  // LOC 2.3.1.1: arrival time in µs since the epoch is the default clock, and WebCodecs needs a timestamp
  return (
    timestampMicros(packet.properties, timescale) ?? Math.round((performance.timeOrigin + performance.now()) * 1000)
  )
}

function startsWith(payload: Uint8Array, prefix: number[]): boolean {
  return prefix.every((byte, i) => payload[i] === byte)
}

function withStartCodes(payload: Uint8Array): Uint8Array | undefined {
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength)
  const output = payload.slice()
  let offset = 0

  while (offset < payload.length) {
    if (offset + 4 > payload.length) return undefined
    const length = view.getUint32(offset)
    output.set(START_CODE, offset)
    offset += 4 + length
  }

  return offset === payload.length ? output : undefined
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const { Timescale } = await import('@/model/property/timescale')
  const { Timestamp } = await import('@/model/property/timestamp')

  describe('annexB', () => {
    test('4-byte lengths become start codes, Annex B passes through, and an overrun is undefined', () => {
      const nal = (length: number, byte: number) => [0, 0, length >> 8, length & 0xff, ...Array(length).fill(byte)]

      expect(annexB(new Uint8Array([...nal(2, 7), ...nal(1, 8)]))).toEqual(
        new Uint8Array([0, 0, 0, 1, 7, 7, 0, 0, 0, 1, 8]),
      )

      // A 300-byte NAL unit's length starts like a 3-byte start code
      expect(annexB(new Uint8Array(nal(300, 9)))?.subarray(0, 4)).toEqual(new Uint8Array(START_CODE))

      for (const annexBPayload of [
        [0, 0, 0, 1, 0x67, 1],
        [0, 0, 1, 0x67, 1],
      ]) {
        const payload = new Uint8Array(annexBPayload)
        expect(annexB(payload)).toBe(payload)
      }

      expect(annexB(new Uint8Array([0, 0, 0, 5, 1, 2]))).toBeUndefined()
      expect(annexB(new Uint8Array([...nal(2, 8), 0, 0]))).toBeUndefined()
    })
  })

  describe('aacDescription', () => {
    test('the AudioSpecificConfig, with the explicit rate outside the table, and nothing for other codecs', () => {
      expect(aacDescription('mp4a.40.2', 48_000, 2)).toEqual(new Uint8Array([0x11, 0x90]))
      // AAC-LC, escape 0xf, 24 bits of 50 kHz, mono
      expect(aacDescription('mp4a.40.2', 50_000, 1)).toEqual(new Uint8Array([0x17, 0x80, 0x61, 0xa8, 0x08]))

      expect(aacDescription('opus', 48_000, 2)).toBeUndefined()
      expect(aacDescription('mp4a.40.2', 48_000, 16)).toBeUndefined()
    })
  })

  describe('chunkType and chunkTimestamp', () => {
    const base = (object: number, properties: import('@/model/common/pair').KeyValuePair[] = []) => ({
      track: 'v1',
      location: new Location(1n, BigInt(object)),
      properties,
    })

    test('key frames come from frame marking, else object 0, and every audio frame is one', () => {
      const marked = (value: number) => [new VideoFrameMarking(new Uint8Array([value])).toKeyValuePair()]

      expect(chunkType(base(3, marked(0x20)), 'video')).toBe('key')
      expect(chunkType(base(0, marked(0)), 'video')).toBe('delta')
      expect(chunkType(base(0), 'video')).toBe('key')
      expect(chunkType(base(1), 'video')).toBe('delta')
      expect(chunkType(base(1), 'audio')).toBe('key')
    })

    test('the timestamp in µs, from the object’s TIMESCALE, else the track’s, else the arrival time', () => {
      const stamped = [new Timestamp(90_000n).toKeyValuePair()]
      expect(chunkTimestamp(base(0, stamped), 90_000)).toBe(1_000_000)
      expect(chunkTimestamp(base(0, [...stamped, new Timescale(1000n).toKeyValuePair()]), 90_000)).toBe(90_000_000)

      const now = (performance.timeOrigin + performance.now()) * 1000
      expect(Math.abs(chunkTimestamp(base(0), undefined) - now)).toBeLessThan(1_000_000)
    })
  })
}
