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
 * Limits the total number of Ranges the peer may have across all Range filter parameters
 * of a single subscription or fetch. Zero, the default when absent, means the peer may
 * not send Range filters at all.
 */
export class MaxFilterRanges implements Parameter {
  static readonly TYPE = SetupOptionType.MaxFilterRanges
  constructor(public readonly value: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxFilterRanges.TYPE, this.value)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxFilterRanges | undefined {
    if (Number(pair.typeValue) !== MaxFilterRanges.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxFilterRanges(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MaxFilterRanges', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new MaxFilterRanges(5n).toKeyValuePair()
      const param = MaxFilterRanges.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(MaxFilterRanges)
      expect(param?.value).toBe(5n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxRequestUpdates, 5n)
      const param = MaxFilterRanges.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
