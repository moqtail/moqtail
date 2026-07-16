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
import { LOCHeaderExtensionId } from './constant'

export class Timestamp {
  static readonly TYPE = LOCHeaderExtensionId.Timestamp
  constructor(public readonly timestamp: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(Timestamp.TYPE, this.timestamp)
  }

  static fromKeyValuePair(pair: KeyValuePair): Timestamp | undefined {
    const type = Number(pair.typeValue)
    if (type === Timestamp.TYPE && typeof pair.value === 'bigint') {
      return new Timestamp(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('TimestampExtensionHeader', () => {
    test('should roundtrip Timestamp', () => {
      const value = 1234567890123456789n
      const pair = new Timestamp(value).toKeyValuePair()
      const header = Timestamp.fromKeyValuePair(pair)
      expect(header).toBeInstanceOf(Timestamp)
      expect(header?.timestamp).toBe(value)
    })
  })
}
