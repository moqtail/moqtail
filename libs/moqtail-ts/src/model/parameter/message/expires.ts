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
 * Time in milliseconds after which the sender will terminate the subscription.
 * A value of 0 means the subscription does not expire or expires at an unknown time.
 * The receiver can extend the subscription by sending a REQUEST_UPDATE.
 */
export class Expires implements Parameter {
  static readonly TYPE = MessageParameterType.Expires

  constructor(public readonly expires: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(Expires.TYPE, this.expires)
  }

  static fromKeyValuePair(pair: KeyValuePair): Expires | undefined {
    if (Number(pair.typeValue) !== Expires.TYPE || typeof pair.value !== 'bigint') return undefined
    return new Expires(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Expires', () => {
    test('roundtrips correctly', () => {
      const orig = new Expires(9999n)
      const pair = orig.toKeyValuePair()
      const parsed = Expires.fromKeyValuePair(pair)
      expect(parsed).toBeInstanceOf(Expires)
      expect(parsed?.expires).toBe(9999n)
    })
    test('accepts 0 (no-expiry sentinel)', () => {
      const orig = new Expires(0n)
      const pair = orig.toKeyValuePair()
      const parsed = Expires.fromKeyValuePair(pair)
      expect(parsed?.expires).toBe(0n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 100n)
      expect(Expires.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
