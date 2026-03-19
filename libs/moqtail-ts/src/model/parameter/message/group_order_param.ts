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
import { GroupOrder } from '../../control/constant'

/**
 * How to prioritize Objects from different groups (Ascending=0x1 or Descending=0x2).
 * Values outside this range are a PROTOCOL_VIOLATION (Original/0x0 is not a valid wire value).
 * If omitted from SUBSCRIBE, the publisher's track preference is used.
 * If omitted from FETCH, Ascending (0x1) is used.
 */
export class GroupOrderParam implements Parameter {
  static readonly TYPE = MessageParameterType.GroupOrder

  constructor(public readonly order: GroupOrder.Ascending | GroupOrder.Descending) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(GroupOrderParam.TYPE, BigInt(this.order))
  }

  static fromKeyValuePair(pair: KeyValuePair): GroupOrderParam | undefined {
    if (Number(pair.typeValue) !== GroupOrderParam.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value !== 1n && pair.value !== 2n) {
      throw new ProtocolViolationError(
        'GroupOrderParam.fromKeyValuePair',
        `GROUP_ORDER must be 1 (Ascending) or 2 (Descending), got ${pair.value}`,
      )
    }
    return new GroupOrderParam(Number(pair.value) as GroupOrder.Ascending | GroupOrder.Descending)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('GroupOrderParam', () => {
    test('roundtrips Ascending', () => {
      const orig = new GroupOrderParam(GroupOrder.Ascending)
      const pair = orig.toKeyValuePair()
      const parsed = GroupOrderParam.fromKeyValuePair(pair)
      expect(parsed?.order).toBe(GroupOrder.Ascending)
    })
    test('roundtrips Descending', () => {
      const orig = new GroupOrderParam(GroupOrder.Descending)
      const pair = orig.toKeyValuePair()
      const parsed = GroupOrderParam.fromKeyValuePair(pair)
      expect(parsed?.order).toBe(GroupOrder.Descending)
    })
    test('fromKeyValuePair throws on value 0 (Original is not valid on wire)', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.GroupOrder, 0n)
      expect(() => GroupOrderParam.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
    test('fromKeyValuePair throws on value outside range', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.GroupOrder, 3n)
      expect(() => GroupOrderParam.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 1n)
      expect(GroupOrderParam.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
