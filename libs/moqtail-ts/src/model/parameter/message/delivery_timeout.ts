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
import { ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Duration in milliseconds the relay SHOULD continue to attempt forwarding Objects.
 * Value MUST be greater than 0; a value of 0 is a PROTOCOL_VIOLATION.
 * This parameter is subscription-specific and SHOULD NOT be forwarded upstream
 * by a relay serving multiple subscriptions for the same track.
 */
export class DeliveryTimeout implements Parameter {
  static readonly TYPE = MessageParameterType.DeliveryTimeout

  constructor(public readonly timeout: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DeliveryTimeout.TYPE, this.timeout)
  }

  static fromKeyValuePair(pair: KeyValuePair): DeliveryTimeout | undefined {
    if (Number(pair.typeValue) !== DeliveryTimeout.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value === 0n) {
      throw new ProtocolViolationError('DeliveryTimeout.fromKeyValuePair', 'DELIVERY_TIMEOUT must be greater than 0')
    }
    return new DeliveryTimeout(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('DeliveryTimeout', () => {
    test('roundtrips correctly', () => {
      const orig = new DeliveryTimeout(0xabcdn)
      const pair = orig.toKeyValuePair()
      const parsed = DeliveryTimeout.fromKeyValuePair(pair)
      expect(parsed).toBeInstanceOf(DeliveryTimeout)
      expect(parsed?.timeout).toBe(0xabcdn)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Expires, 100n)
      expect(DeliveryTimeout.fromKeyValuePair(pair)).toBeUndefined()
    })
    test('fromKeyValuePair throws on value 0', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 0n)
      expect(() => DeliveryTimeout.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
  })
}
