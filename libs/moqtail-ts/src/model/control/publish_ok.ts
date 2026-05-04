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

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { KeyValuePair } from '../common/pair'
import { Location } from '../common/location'
import { LengthExceedsMaxError } from '../error/error'
import { ControlMessageType, FilterType, GroupOrder } from './constant'
import { MessageParameter, MessageParameters } from '../parameter/message_parameter'
import { Forward } from '../parameter/message/forward'
import { SubscriberPriority } from '../parameter/message/subscriber_priority'
import { GroupOrderParam } from '../parameter/message/group_order_param'
import { SubscriptionFilter } from '../parameter/message/subscription_filter'
import { DeliveryTimeout } from '../parameter/message/delivery_timeout'

export class PublishOk {
  constructor(
    public readonly requestId: bigint,
    public readonly parameters: MessageParameter[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.PublishOk
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishOk)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)

    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param.toKeyValuePair())
    }

    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishOk::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): PublishOk {
    const requestId = buf.getVI()
    const paramCount = buf.getVI()
    const kvps: KeyValuePair[] = []
    for (let i = 0; i < paramCount; i++) {
      kvps.push(buf.getKeyValuePair())
    }
    const parameters = MessageParameters.fromKeyValuePairs(kvps)

    return new PublishOk(requestId, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('PublishOk', () => {
    test('roundtrip with no parameters', () => {
      const publishOk = new PublishOk(12345n, [])

      const frozen = publishOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishOk.requestId)
      expect(deserialized.parameters.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with parameters', () => {
      const parameters: MessageParameter[] = [
        new Forward(true),
        new SubscriberPriority(100),
        new GroupOrderParam(GroupOrder.Ascending),
        new SubscriptionFilter(FilterType.LatestObject, undefined, undefined),
      ]
      const publishOk = new PublishOk(12345n, parameters)

      const frozen = publishOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishOk.requestId)
      expect(deserialized.parameters.length).toBe(parameters.length)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with AbsoluteRange subscription filter', () => {
      const parameters: MessageParameter[] = [
        new SubscriptionFilter(FilterType.AbsoluteRange, new Location(5n, 10n), 20n),
        new DeliveryTimeout(5000n),
      ]
      const publishOk = new PublishOk(789n, parameters)

      const frozen = publishOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(789n)
      expect(deserialized.parameters.length).toBe(2)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const publishOk = new PublishOk(12345n, [new Forward(true)])
      const serialized = publishOk.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishOk.requestId)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const publishOk = new PublishOk(12345n, [new Forward(true)])
      const serialized = publishOk.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        PublishOk.parsePayload(frozen)
      }).toThrow()
    })
  })
}
