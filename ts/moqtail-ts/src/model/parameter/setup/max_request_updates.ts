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
 * Maximum number of outstanding REQUEST_UPDATEs per request stream: an update counts
 * until its REQUEST_OK or REQUEST_ERROR arrives. Zero, the default when absent, means
 * no limit. Receiving one past the limit closes the session with TOO_MANY_REQUEST_UPDATES.
 */
export class MaxRequestUpdates implements Parameter {
  static readonly TYPE = SetupOptionType.MaxRequestUpdates
  constructor(public readonly value: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxRequestUpdates.TYPE, this.value)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxRequestUpdates | undefined {
    if (Number(pair.typeValue) !== MaxRequestUpdates.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxRequestUpdates(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MaxRequestUpdates', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new MaxRequestUpdates(9n).toKeyValuePair()
      const param = MaxRequestUpdates.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(MaxRequestUpdates)
      expect(param?.value).toBe(9n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxFilterRanges, 9n)
      const param = MaxRequestUpdates.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
