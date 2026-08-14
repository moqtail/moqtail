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
 * Duration in milliseconds the relay SHOULD continue to attempt forwarding a Subgroup.
 * A value of 0 means no timeout is set (§8).
 */
export class SubgroupDeliveryTimeout implements Parameter {
  static readonly TYPE = MessageParameterType.SubgroupDeliveryTimeout

  constructor(public readonly timeout: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(SubgroupDeliveryTimeout.TYPE, this.timeout)
  }

  static fromKeyValuePair(pair: KeyValuePair): SubgroupDeliveryTimeout | undefined {
    if (Number(pair.typeValue) !== SubgroupDeliveryTimeout.TYPE || typeof pair.value !== 'bigint') return undefined
    return new SubgroupDeliveryTimeout(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SubgroupDeliveryTimeout', () => {
    test('roundtrips correctly', () => {
      const pair = new SubgroupDeliveryTimeout(2500n).toKeyValuePair()
      expect(SubgroupDeliveryTimeout.fromKeyValuePair(pair)?.timeout).toBe(2500n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 100n)
      expect(SubgroupDeliveryTimeout.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
