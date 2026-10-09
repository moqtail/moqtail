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

import { KeyValuePair } from '../../common/pair'
import { SetupOptionType } from '../constant'
import { Parameter } from '../parameter'

/**
 * §10.3.1.6. The total number of Ranges the peer may send across all Range
 * Filter parameters of one subscription or fetch.
 *
 * Omitting this option is the same as sending 0, and 0 prohibits Range Filters
 * outright rather than meaning unlimited.
 */
export class MaxFilterRanges implements Parameter {
  static readonly TYPE = SetupOptionType.MaxFilterRanges
  constructor(public readonly maxRanges: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxFilterRanges.TYPE, this.maxRanges)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxFilterRanges | undefined {
    if (Number(pair.typeValue) !== MaxFilterRanges.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxFilterRanges(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MaxFilterRanges', () => {
    test('round-trips through its key-value pair', () => {
      const param = MaxFilterRanges.fromKeyValuePair(new MaxFilterRanges(16n).toKeyValuePair())
      expect(param).toBeInstanceOf(MaxFilterRanges)
      expect(param?.maxRanges).toBe(16n)
    })

    test('0 is a real value, and is the default when the option is absent', () => {
      expect(MaxFilterRanges.fromKeyValuePair(new MaxFilterRanges(0n).toKeyValuePair())?.maxRanges).toBe(0n)
    })

    test('fromKeyValuePair returns undefined for another setup option', () => {
      const other = KeyValuePair.tryNewVarInt(SetupOptionType.MaxAuthTokenCacheSize, 123n)
      expect(MaxFilterRanges.fromKeyValuePair(other)).toBeUndefined()
    })
  })
}
