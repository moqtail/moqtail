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
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'
import { MessageParameter, MessageParameters } from '../parameter/message_parameter'
import { TrackExtension } from '../extension_header/track_extension'

import { Location } from '../common/location'
import { GroupOrder } from './constant'
import { DeliveryTimeoutExtension } from '../extension_header/track_extension'
import { Expires } from '../parameter/message/expires'
import { LargestObject } from '../parameter/message/largest_object'
import { GroupOrderParam } from '../parameter/message/group_order_param'
import { DeliveryTimeout } from '../parameter/message/delivery_timeout'

export class SubscribeOk {
  requestId: bigint
  trackAlias: bigint
  parameters: MessageParameter[]
  trackExtensions: TrackExtension[]

  private constructor(
    requestId: bigint,
    trackAlias: bigint,
    parameters: MessageParameter[],
    trackExtensions: TrackExtension[],
  ) {
    this.requestId = requestId
    this.trackAlias = trackAlias
    this.parameters = parameters
    this.trackExtensions = trackExtensions
  }

  static create(
    requestId: bigint,
    trackAlias: bigint,
    parameters: MessageParameter[],
    trackExtensions: TrackExtension[] = [],
  ): SubscribeOk {
    return new SubscribeOk(requestId, trackAlias, parameters, trackExtensions)
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.SubscribeOk)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.trackAlias)
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param.toKeyValuePair())
    }
    TrackExtension.serializeInto(this.trackExtensions, payload)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('SubscribeOk::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): SubscribeOk {
    const requestId = buf.getVI()
    const trackAlias = buf.getVI()
    const paramCount = buf.getNumberVI()
    const rawParams = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      rawParams[i] = buf.getKeyValuePair()
    }
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)
    const trackExtensions = TrackExtension.deserializeAll(buf)
    return new SubscribeOk(requestId, trackAlias, parameters, trackExtensions)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SubscribeOk', () => {
    test('roundtrip', () => {
      const requestId = 145136n
      const trackAlias = 999n
      const largestLocation = new Location(34n, 0n)
      const parameters = [
        new Expires(16n),
        new GroupOrderParam(GroupOrder.Ascending),
        new LargestObject(largestLocation),
        new DeliveryTimeout(100n),
      ]
      const subscribeOk = SubscribeOk.create(requestId, trackAlias, parameters)
      const frozen = subscribeOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = SubscribeOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(subscribeOk.requestId)
      expect(deserialized.trackAlias).toBe(subscribeOk.trackAlias)
      expect(deserialized.parameters.length).toBe(4)
      expect(deserialized.trackExtensions.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with track extensions', () => {
      const subscribeOk = SubscribeOk.create(
        145136n,
        999n,
        [new Expires(16n), new LargestObject(new Location(34n, 0n))],
        [new DeliveryTimeoutExtension(5000n)],
      )
      const frozen = subscribeOk.serialize()
      frozen.getVI() // message type
      const msgLength = frozen.getU16()
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = SubscribeOk.parsePayload(payload)
      expect(deserialized.trackExtensions.length).toBe(1)
      expect(deserialized.trackExtensions[0]).toBeInstanceOf(DeliveryTimeoutExtension)
      expect(payload.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 145136n
      const trackAlias = 999n
      const subscribeOk = SubscribeOk.create(requestId, trackAlias, [
        new Expires(16n),
        new LargestObject(new Location(34n, 0n)),
        new DeliveryTimeout(100n),
      ])
      const serialized = subscribeOk.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = SubscribeOk.parsePayload(payload)
      expect(deserialized.requestId).toBe(subscribeOk.requestId)
      expect(deserialized.trackAlias).toBe(subscribeOk.trackAlias)
      expect(deserialized.parameters.length).toBe(3)
      expect(payload.remaining).toBe(0)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const subscribeOk = SubscribeOk.create(145136n, 999n, [
        new Expires(16n),
        new LargestObject(new Location(34n, 0n)),
      ])
      const serialized = subscribeOk.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        SubscribeOk.parsePayload(frozen)
      }).toThrow()
    })
  })
}
