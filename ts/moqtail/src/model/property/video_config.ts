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

export class VideoConfig {
  static readonly TYPE = LOCPropertyId.VideoConfig
  constructor(public readonly config: Uint8Array) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(VideoConfig.TYPE, this.config)
  }

  static fromKeyValuePair(pair: KeyValuePair): VideoConfig | undefined {
    const type = Number(pair.typeValue)
    if (type === VideoConfig.TYPE && pair.value instanceof Uint8Array) {
      return new VideoConfig(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('VideoConfigProperty', () => {
    test('should roundtrip VideoConfig', () => {
      const value = new Uint8Array([1, 2, 3, 4])
      const pair = new VideoConfig(value).toKeyValuePair()
      const header = VideoConfig.fromKeyValuePair(pair)
      expect(header).toBeInstanceOf(VideoConfig)
      expect(header?.config).toBeInstanceOf(Uint8Array)
      expect(Array.from(header!.config)).toEqual(Array.from(value))
    })
  })
}
