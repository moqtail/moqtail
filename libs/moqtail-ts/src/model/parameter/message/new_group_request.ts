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
 * The largest Group ID in the Track known by the subscriber, plus 1.
 * A value of 0 indicates the subscriber has no Group information for the Track.
 * Only valid for tracks with the DYNAMIC_GROUPS extension (value 1).
 */
export class NewGroupRequest implements Parameter {
  static readonly TYPE = MessageParameterType.NewGroupRequest

  constructor(public readonly group: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(NewGroupRequest.TYPE, this.group)
  }

  static fromKeyValuePair(pair: KeyValuePair): NewGroupRequest | undefined {
    if (Number(pair.typeValue) !== NewGroupRequest.TYPE || typeof pair.value !== 'bigint') return undefined
    return new NewGroupRequest(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('NewGroupRequest', () => {
    test('roundtrips correctly', () => {
      const orig = new NewGroupRequest(7n)
      const pair = orig.toKeyValuePair()
      const parsed = NewGroupRequest.fromKeyValuePair(pair)
      expect(parsed?.group).toBe(7n)
    })
    test('accepts 0 (no group info)', () => {
      const orig = new NewGroupRequest(0n)
      const pair = orig.toKeyValuePair()
      const parsed = NewGroupRequest.fromKeyValuePair(pair)
      expect(parsed?.group).toBe(0n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 100n)
      expect(NewGroupRequest.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
