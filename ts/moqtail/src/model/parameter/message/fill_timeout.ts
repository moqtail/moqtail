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
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Duration in milliseconds a relay SHOULD wait for objects it does not yet hold before
 * ending the FETCH. FETCH only (§10.2.5).
 */
export class FillTimeout implements Parameter {
  static readonly TYPE = MessageParameterType.FillTimeout

  constructor(public readonly timeout: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(FillTimeout.TYPE, this.timeout)
  }

  static fromKeyValuePair(pair: KeyValuePair): FillTimeout | undefined {
    if (Number(pair.typeValue) !== FillTimeout.TYPE || typeof pair.value !== 'bigint') return undefined
    return new FillTimeout(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('FillTimeout', () => {
    test('roundtrips correctly', () => {
      const pair = new FillTimeout(3000n).toKeyValuePair()
      expect(FillTimeout.fromKeyValuePair(pair)?.timeout).toBe(3000n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 100n)
      expect(FillTimeout.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
