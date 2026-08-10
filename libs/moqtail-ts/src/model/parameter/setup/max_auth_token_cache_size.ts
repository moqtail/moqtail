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

export class MaxAuthTokenCacheSize implements Parameter {
  static readonly TYPE = SetupOptionType.MaxAuthTokenCacheSize
  constructor(public readonly maxSize: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxAuthTokenCacheSize.TYPE, this.maxSize)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxAuthTokenCacheSize | undefined {
    if (Number(pair.typeValue) !== MaxAuthTokenCacheSize.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxAuthTokenCacheSize(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MaxAuthTokenCacheSize', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new MaxAuthTokenCacheSize(123n).toKeyValuePair()
      const param = MaxAuthTokenCacheSize.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(MaxAuthTokenCacheSize)
      expect(param?.maxSize).toBe(123n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxRequestId, 123n)
      const param = MaxAuthTokenCacheSize.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
