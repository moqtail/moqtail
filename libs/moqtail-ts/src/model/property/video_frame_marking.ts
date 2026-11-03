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
import { LOCPropertyId } from './constant'

/** The I (independent frame) bit of the RFC 9626 flag octet. */
const INDEPENDENT = 0x20

export class VideoFrameMarking {
  static readonly TYPE = LOCPropertyId.VideoFrameMarking
  /** The RFC 9626 frame-marking octets (1 to 4), without the length prefix. */
  constructor(public readonly value: Uint8Array) {}

  /** True if the frame decodes without temporally prior frames (the I bit). */
  get independent(): boolean {
    return ((this.value[0] ?? 0) & INDEPENDENT) !== 0
  }

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(VideoFrameMarking.TYPE, this.value)
  }

  static fromKeyValuePair(pair: KeyValuePair): VideoFrameMarking | undefined {
    const type = Number(pair.typeValue)
    if (type === VideoFrameMarking.TYPE && pair.value instanceof Uint8Array) {
      return new VideoFrameMarking(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('VideoFrameMarkingProperty', () => {
    test('should roundtrip VideoFrameMarking', () => {
      const value = new Uint8Array([0xe0, 0x01, 0x02])
      const pair = new VideoFrameMarking(value).toKeyValuePair()
      const header = VideoFrameMarking.fromKeyValuePair(pair)
      expect(header).toBeInstanceOf(VideoFrameMarking)
      expect(Array.from(header!.value)).toEqual(Array.from(value))
    })

    test('independent reads only the I bit and tolerates an empty value', () => {
      expect(new VideoFrameMarking(new Uint8Array([0x20])).independent).toBe(true)
      expect(new VideoFrameMarking(new Uint8Array([0xdf])).independent).toBe(false)
      expect(new VideoFrameMarking(new Uint8Array([])).independent).toBe(false)
    })
  })
}
