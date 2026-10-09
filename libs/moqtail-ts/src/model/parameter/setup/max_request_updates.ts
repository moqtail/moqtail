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
 * §10.3.1.7. How many REQUEST_UPDATEs the peer may leave outstanding on a single
 * request stream -- outstanding meaning sent and not yet answered by a REQUEST_OK
 * or REQUEST_ERROR, each of which restores one credit.
 *
 * 0 is the default and means no limit, which is the reverse of
 * {@link MaxFilterRanges}, where 0 is a prohibition. Exceeding the limit closes the
 * session with TOO_MANY_REQUEST_UPDATES.
 */
export class MaxRequestUpdates implements Parameter {
  static readonly TYPE = SetupOptionType.MaxRequestUpdates
  constructor(public readonly maxUpdates: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxRequestUpdates.TYPE, this.maxUpdates)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxRequestUpdates | undefined {
    if (Number(pair.typeValue) !== MaxRequestUpdates.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxRequestUpdates(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MaxRequestUpdates', () => {
    test('round-trips through its key-value pair', () => {
      const param = MaxRequestUpdates.fromKeyValuePair(new MaxRequestUpdates(4n).toKeyValuePair())
      expect(param).toBeInstanceOf(MaxRequestUpdates)
      expect(param?.maxUpdates).toBe(4n)
    })

    test('0 is a real value and means no limit, unlike MAX_FILTER_RANGES', () => {
      expect(MaxRequestUpdates.fromKeyValuePair(new MaxRequestUpdates(0n).toKeyValuePair())?.maxUpdates).toBe(0n)
    })

    test('fromKeyValuePair returns undefined for another setup option', () => {
      const other = KeyValuePair.tryNewVarInt(SetupOptionType.MaxFilterRanges, 4n)
      expect(MaxRequestUpdates.fromKeyValuePair(other)).toBeUndefined()
    })
  })
}
