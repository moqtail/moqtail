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

import { ByteBuffer } from '../../common'
import { KeyValuePair } from '../../common/pair'
import { ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'
import type { MessageParameter } from '../message_parameter'
import {
  MessageParameters,
  deserializeMessageParameterKvpsUntilEmpty,
  serializeMessageParameterKvps,
} from '../message_parameter'

/** The only parameters that may override anything for the fill fetch stream. */
const ALLOWED: readonly MessageParameterType[] = [
  MessageParameterType.FillTimeout,
  MessageParameterType.SubscriberPriority,
  MessageParameterType.GroupOrder,
]

/**
 * Overrides for the fill fetch stream. An omitted parameter keeps the value it
 * has for the live subscription. Ignored without a fill filter type.
 */
export class FillParameters implements Parameter {
  static readonly TYPE = MessageParameterType.FillParameters

  constructor(public readonly parameters: MessageParameter[]) {}

  toKeyValuePair(): KeyValuePair {
    const pairs = this.parameters.map((p) => p.toKeyValuePair())
    for (const pair of pairs) {
      if (!ALLOWED.includes(Number(pair.typeValue))) {
        throw new ProtocolViolationError(
          'FillParameters.toKeyValuePair',
          `parameter type 0x${pair.typeValue.toString(16)} is not allowed inside FILL_PARAMETERS`,
        )
      }
    }
    return KeyValuePair.tryNewBytes(FillParameters.TYPE, serializeMessageParameterKvps(pairs).toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): FillParameters | undefined {
    if (Number(pair.typeValue) !== FillParameters.TYPE || !(pair.value instanceof Uint8Array)) return undefined

    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const pairs = deserializeMessageParameterKvpsUntilEmpty(buf.freeze())
    for (const inner of pairs) {
      if (!ALLOWED.includes(Number(inner.typeValue))) {
        throw new ProtocolViolationError(
          'FillParameters.fromKeyValuePair',
          `parameter type 0x${inner.typeValue.toString(16)} is not allowed inside FILL_PARAMETERS`,
        )
      }
    }
    return new FillParameters(MessageParameters.fromKeyValuePairs(pairs))
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { FillTimeout } = await import('./fill_timeout')
  const { SubscriberPriority } = await import('./subscriber_priority')
  const { GroupOrderParam } = await import('./group_order_param')
  const { GroupOrder } = await import('../../control/constant')
  const { Forward } = await import('./forward')

  describe('FillParameters', () => {
    test('roundtrips its permitted parameters', () => {
      const orig = new FillParameters([
        new FillTimeout(3000n),
        new SubscriberPriority(10),
        new GroupOrderParam(GroupOrder.Descending),
      ])
      const parsed = FillParameters.fromKeyValuePair(orig.toKeyValuePair())
      expect(parsed).toEqual(orig)
    })

    test('nests a delta-encoded parameter block in its value', async () => {
      const { serializeMessageParameterKvps } = await import('../message_parameter')
      const params = [new FillParameters([new FillTimeout(3000n)])]
      const wire = serializeMessageParameterKvps(params.map((p) => p.toKeyValuePair())).toUint8Array()

      expect(wire).toEqual(new Uint8Array([0x23, 0x03, 0x0a, 0x8b, 0xb8]))
    })

    test('rejects a parameter it cannot override', () => {
      // FORWARD belongs to the subscription, not to its fill.
      expect(() => new FillParameters([new Forward(true)]).toKeyValuePair()).toThrow(ProtocolViolationError)
    })
  })
}
