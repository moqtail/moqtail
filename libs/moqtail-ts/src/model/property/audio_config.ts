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

import { KeyValuePair } from '../common/pair'
import { LOCPropertyId } from './constant'

export class AudioConfig {
  static readonly TYPE = LOCPropertyId.AudioConfig
  constructor(public readonly config: Uint8Array) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(AudioConfig.TYPE, this.config)
  }

  static fromKeyValuePair(pair: KeyValuePair): AudioConfig | undefined {
    const type = Number(pair.typeValue)
    if (type === AudioConfig.TYPE && pair.value instanceof Uint8Array) {
      return new AudioConfig(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('AudioConfigProperty', () => {
    test('should roundtrip AudioConfig', () => {
      const value = new Uint8Array([1, 2, 3, 4])
      const pair = new AudioConfig(value).toKeyValuePair()
      const header = AudioConfig.fromKeyValuePair(pair)
      expect(header).toBeInstanceOf(AudioConfig)
      expect(header?.config).toBeInstanceOf(Uint8Array)
      expect(Array.from(header!.config)).toEqual(Array.from(value))
    })
  })
}
