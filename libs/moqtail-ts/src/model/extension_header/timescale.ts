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
import { LOCHeaderExtensionId } from './constant'

export class Timescale {
  static readonly TYPE = LOCHeaderExtensionId.Timescale

  constructor(public readonly timescale: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(Timescale.TYPE, this.timescale)
  }

  static fromKeyValuePair(pair: KeyValuePair): Timescale | undefined {
    const type = Number(pair.typeValue)
    if (type === Timescale.TYPE && typeof pair.value === 'bigint') {
      return new Timescale(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('TimescaleExtensionHeader', () => {
    test('should roundtrip Timescale', () => {
      const value = 90000n
      const pair = new Timescale(value).toKeyValuePair()
      const parsed = Timescale.fromKeyValuePair(pair)
      expect(parsed).toBeInstanceOf(Timescale)
      expect(parsed?.timescale).toBe(value)
    })

    test('should return undefined for a different type', () => {
      const pair = KeyValuePair.tryNewVarInt(LOCHeaderExtensionId.AudioLevel, 1n)
      expect(Timescale.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
