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
 * Duration in milliseconds the relay SHOULD continue to attempt forwarding an Object.
 * A value of 0 means no timeout is set (§8).
 * This parameter is subscription-specific and SHOULD NOT be forwarded upstream
 * by a relay serving multiple subscriptions for the same track.
 */
export class ObjectDeliveryTimeout implements Parameter {
  static readonly TYPE = MessageParameterType.ObjectDeliveryTimeout

  constructor(public readonly timeout: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(ObjectDeliveryTimeout.TYPE, this.timeout)
  }

  static fromKeyValuePair(pair: KeyValuePair): ObjectDeliveryTimeout | undefined {
    if (Number(pair.typeValue) !== ObjectDeliveryTimeout.TYPE || typeof pair.value !== 'bigint') return undefined
    return new ObjectDeliveryTimeout(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('ObjectDeliveryTimeout', () => {
    test('roundtrips correctly', () => {
      const orig = new ObjectDeliveryTimeout(0xabcdn)
      const pair = orig.toKeyValuePair()
      const parsed = ObjectDeliveryTimeout.fromKeyValuePair(pair)
      expect(parsed).toBeInstanceOf(ObjectDeliveryTimeout)
      expect(parsed?.timeout).toBe(0xabcdn)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Expires, 100n)
      expect(ObjectDeliveryTimeout.fromKeyValuePair(pair)).toBeUndefined()
    })
    // §8: draft-16 rejected 0; draft-18 reads it as "no timeout".
    test('a value of 0 means no timeout', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 0n)
      expect(ObjectDeliveryTimeout.fromKeyValuePair(pair)?.timeout).toBe(0n)
    })
  })
}
