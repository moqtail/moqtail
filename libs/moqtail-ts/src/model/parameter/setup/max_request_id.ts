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

import { KeyValuePair } from '../../common/pair'
import { SetupOptionType } from '../constant'
import { Parameter } from '../parameter'

export class MaxRequestId implements Parameter {
  static readonly TYPE = SetupOptionType.MaxRequestId
  constructor(public readonly maxId: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxRequestId.TYPE, this.maxId)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxRequestId | undefined {
    if (Number(pair.typeValue) !== MaxRequestId.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxRequestId(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MaxRequestId', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new MaxRequestId(42n).toKeyValuePair()
      const param = MaxRequestId.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(MaxRequestId)
      expect(param?.maxId).toBe(42n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxAuthTokenCacheSize, 42n)
      const param = MaxRequestId.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
