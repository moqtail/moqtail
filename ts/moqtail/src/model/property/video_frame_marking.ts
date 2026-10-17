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

export class VideoFrameMarking {
  static readonly TYPE = LOCPropertyId.VideoFrameMarking
  constructor(public readonly value: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(VideoFrameMarking.TYPE, this.value)
  }

  static fromKeyValuePair(pair: KeyValuePair): VideoFrameMarking | undefined {
    const type = Number(pair.typeValue)
    if (type === VideoFrameMarking.TYPE && typeof pair.value === 'bigint') {
      return new VideoFrameMarking(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('VideoFrameMarkingProperty', () => {
    test('should roundtrip VideoFrameMarking', () => {
      const value = 9876543210987654321n
      const pair = new VideoFrameMarking(value).toKeyValuePair()
      const header = VideoFrameMarking.fromKeyValuePair(pair)
      expect(header).toBeInstanceOf(VideoFrameMarking)
      expect(header?.value).toBe(value)
    })
  })
}
