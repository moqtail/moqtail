/**
 * Copyright 2025 The MOQtail Authors
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

import { KeyValuePair } from '../common/pair'
import { Timestamp } from './timestamp'
import { Timescale } from './timescale'
import { VideoFrameMarking } from './video_frame_marking'
import { AudioLevel } from './audio_level'
import { VideoConfig } from './video_config'
import { AudioConfig } from './audio_config'

export type LOCProperty = Timestamp | Timescale | VideoFrameMarking | AudioLevel | VideoConfig | AudioConfig

export namespace LOCProperty {
  export function fromKeyValuePair(pair: KeyValuePair): LOCProperty | undefined {
    return (
      Timestamp.fromKeyValuePair(pair) ||
      Timescale.fromKeyValuePair(pair) ||
      VideoFrameMarking.fromKeyValuePair(pair) ||
      AudioLevel.fromKeyValuePair(pair) ||
      VideoConfig.fromKeyValuePair(pair) ||
      AudioConfig.fromKeyValuePair(pair)
    )
  }

  export function toKeyValuePair(header: LOCProperty): KeyValuePair {
    return header.toKeyValuePair()
  }

  export function isTimestamp(header: LOCProperty): header is Timestamp {
    return header instanceof Timestamp
  }
  export function isTimescale(header: LOCProperty): header is Timescale {
    return header instanceof Timescale
  }
  export function isVideoFrameMarking(header: LOCProperty): header is VideoFrameMarking {
    return header instanceof VideoFrameMarking
  }
  export function isAudioLevel(header: LOCProperty): header is AudioLevel {
    return header instanceof AudioLevel
  }
  export function isVideoConfig(header: LOCProperty): header is VideoConfig {
    return header instanceof VideoConfig
  }
  export function isAudioConfig(header: LOCProperty): header is AudioConfig {
    return header instanceof AudioConfig
  }
}

export class LOCProperties {
  private kvps: KeyValuePair[] = []

  addTimestamp(timestamp: bigint | number): this {
    this.kvps.push(new Timestamp(BigInt(timestamp)).toKeyValuePair())
    return this
  }
  addTimescale(timescale: bigint | number): this {
    this.kvps.push(new Timescale(BigInt(timescale)).toKeyValuePair())
    return this
  }
  /** Marks a whole, non-scalable frame (RFC 9626 short form): S and E set, I set on a keyframe. */
  addVideoFrameMarking(isKey: boolean): this {
    this.kvps.push(new VideoFrameMarking(new Uint8Array([isKey ? 0xe0 : 0xc0])).toKeyValuePair())
    return this
  }
  addAudioLevel(audioLevel: bigint | number): this {
    this.kvps.push(new AudioLevel(BigInt(audioLevel)).toKeyValuePair())
    return this
  }
  addVideoConfig(config: Uint8Array): this {
    this.kvps.push(new VideoConfig(config).toKeyValuePair())
    return this
  }
  addAudioConfig(config: Uint8Array): this {
    this.kvps.push(new AudioConfig(config).toKeyValuePair())
    return this
  }
  addRaw(pair: KeyValuePair): this {
    this.kvps.push(pair)
    return this
  }
  build(): KeyValuePair[] {
    return this.kvps
  }
  static fromKeyValuePairs(kvps: KeyValuePair[]): LOCProperty[] {
    const result: LOCProperty[] = []
    for (const kvp of kvps) {
      const parsed = LOCProperty.fromKeyValuePair(kvp)
      if (parsed) result.push(parsed)
    }
    return result
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('LOCProperties', () => {
    test('build and fromKeyValuePairs returns correct parameters', () => {
      const timestamp = 11223344n
      const audioLevel = 99n
      const videoConfig = new Uint8Array([3, 1, 2, 4, 5, 6, 2, 5, 3])
      const audioConfig = new Uint8Array([0x11, 0x90])
      const kvps = new LOCProperties()
        .addTimestamp(timestamp)
        .addVideoFrameMarking(true)
        .addAudioLevel(audioLevel)
        .addVideoConfig(videoConfig)
        .addAudioConfig(audioConfig)
        .build()
      const parsed = LOCProperties.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(5)
      expect(parsed[0] && LOCProperty.isTimestamp(parsed[0]) && parsed[0].timestamp === timestamp).toBe(true)
      expect(parsed[1] && LOCProperty.isVideoFrameMarking(parsed[1]) && parsed[1].independent).toBe(true)
      expect(parsed[2] && LOCProperty.isAudioLevel(parsed[2]) && parsed[2].audioLevel === audioLevel).toBe(true)
      expect(parsed[3] && LOCProperty.isVideoConfig(parsed[3]) && parsed[3].config === videoConfig).toBe(true)
      expect(parsed[4] && LOCProperty.isAudioConfig(parsed[4]) && parsed[4].config === audioConfig).toBe(true)
    })

    test('addVideoFrameMarking writes the RFC 9626 short form', () => {
      const [key, delta] = new LOCProperties().addVideoFrameMarking(true).addVideoFrameMarking(false).build()
      expect(key?.typeValue).toBe(0x09n)
      expect(Array.from(key?.value as Uint8Array)).toEqual([0xe0])
      expect(Array.from(delta?.value as Uint8Array)).toEqual([0xc0])
    })

    test('fromKeyValuePairs skips unknown parameter', () => {
      const unknown = KeyValuePair.tryNewVarInt(100, 31n)
      const timestamp = 5927836n
      const audioLevel = 99n
      const kvps = new LOCProperties()
        .addTimestamp(timestamp)
        .addRaw(unknown)
        .addVideoFrameMarking(false)
        .addAudioLevel(audioLevel)
        .build()
      const parsed = LOCProperties.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(3)
      expect(parsed[0] && LOCProperty.isTimestamp(parsed[0]) && parsed[0].timestamp === timestamp).toBe(true)
      expect(parsed[1] && LOCProperty.isVideoFrameMarking(parsed[1]) && !parsed[1].independent).toBe(true)
      expect(parsed[2] && LOCProperty.isAudioLevel(parsed[2]) && parsed[2].audioLevel === audioLevel).toBe(true)
    })
  })
}
