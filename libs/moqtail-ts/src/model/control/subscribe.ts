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
import { Location } from '../common/location'
import { Tuple } from '../common/tuple'
import { KeyValuePair } from '../common/pair'
import { ControlMessageType, FilterType, GroupOrder } from '../control/constant'
import { FullTrackName } from '../data'
import { MessageParameter } from '../parameter/message_parameter'
import { SubscriptionFilter } from '../parameter/message/subscription_filter'
import { Forward } from '../parameter/message/forward'
import { GroupOrderParam } from '../parameter/message/group_order_param'

export class Subscribe {
  private constructor(
    public requestId: bigint,
    public fullTrackName: FullTrackName,
    public parameters: MessageParameter[],
  ) {}

  static newNextGroupStart(requestId: bigint, fullTrackName: FullTrackName, parameters: MessageParameter[]): Subscribe {
    return new Subscribe(requestId, fullTrackName, [...parameters, new SubscriptionFilter(FilterType.NextGroupStart)])
  }

  static newLatestObject(requestId: bigint, fullTrackName: FullTrackName, parameters: MessageParameter[]): Subscribe {
    return new Subscribe(requestId, fullTrackName, [...parameters, new SubscriptionFilter(FilterType.LatestObject)])
  }

  static newAbsoluteStart(
    requestId: bigint,
    fullTrackName: FullTrackName,
    startLocation: Location,
    parameters: MessageParameter[],
  ): Subscribe {
    return new Subscribe(requestId, fullTrackName, [
      ...parameters,
      new SubscriptionFilter(FilterType.AbsoluteStart, startLocation),
    ])
  }

  static newAbsoluteRange(
    requestId: bigint,
    fullTrackName: FullTrackName,
    startLocation: Location,
    endGroup: bigint,
    parameters: MessageParameter[],
  ): Subscribe {
    if (endGroup < startLocation.group) {
      throw new Error('End Group must be >= Start Group')
    }
    return new Subscribe(requestId, fullTrackName, [
      ...parameters,
      new SubscriptionFilter(FilterType.AbsoluteRange, startLocation, endGroup),
    ])
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Subscribe)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putBytes(this.fullTrackName.serialize().toUint8Array())

    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putBytes(param.toKeyValuePair().serialize().toUint8Array())
    }

    const payloadBytes = payload.toUint8Array()
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): Subscribe {
    const requestId = buf.getVI()
    const fullTrackName = buf.getFullTrackName()

    const paramCount = Number(buf.getVI())
    const parameters: MessageParameter[] = []
    for (let i = 0; i < paramCount; i++) {
      const kvp = KeyValuePair.deserialize(buf)
      const param = MessageParameter.fromKeyValuePair(kvp)
      if (param !== undefined) parameters.push(param)
    }

    return new Subscribe(requestId, fullTrackName, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  function buildTestSubscribe(): Subscribe {
    return Subscribe.newAbsoluteRange(
      128242n,
      FullTrackName.tryNew('track/namespace', 'trackName'),
      new Location(81n, 81n),
      100n,
      [new Forward(true), new GroupOrderParam(GroupOrder.Ascending)],
    )
  }

  describe('Subscribe', () => {
    it('should roundtrip correctly', () => {
      const subscribe = buildTestSubscribe()
      const serialized = subscribe.serialize()

      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Subscribe))

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining)

      const deserialized = Subscribe.parsePayload(buf)
      expect(deserialized).toEqual(subscribe)
      expect(buf.remaining).toBe(0)
    })

    it('should roundtrip with excess trailing bytes', () => {
      const subscribe = buildTestSubscribe()
      const serialized = subscribe.serialize()
      const extra = new Uint8Array([...serialized.toUint8Array(), 9, 1, 1])

      const buf = new ByteBuffer()
      buf.putBytes(extra)

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Subscribe))

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining - 3)

      const deserialized = Subscribe.parsePayload(buf)
      expect(deserialized).toEqual(subscribe)

      const trailing = buf.toUint8Array().slice(buf.offset)
      expect(Array.from(trailing)).toEqual([9, 1, 1])
    })

    describe('Subscribe Constructors', () => {
      it('should create a Subscribe with AbsoluteRange filter in parameters', () => {
        const subscribe = Subscribe.newAbsoluteRange(
          128242n,
          FullTrackName.tryNew('track/namespace', 'trackName'),
          new Location(81n, 81n),
          100n,
          [],
        )

        const filter = subscribe.parameters.find((p): p is SubscriptionFilter => p instanceof SubscriptionFilter)
        expect(filter?.filterType).toBe(FilterType.AbsoluteRange)
        expect(filter?.startLocation).toEqual(new Location(81n, 81n))
        expect(filter?.endGroup).toBe(100n)
      })

      it('should throw an error if EndGroup < StartGroup', () => {
        expect(() =>
          Subscribe.newAbsoluteRange(
            128242n,
            FullTrackName.tryNew('track/namespace', 'trackName'),
            new Location(81n, 81n),
            80n,
            [],
          ),
        ).toThrow('End Group must be >= Start Group')
      })
    })

    it('should handle empty parameters', () => {
      const subscribe = Subscribe.newLatestObject(128242n, FullTrackName.tryNew('track/namespace', 'trackName'), [])

      const serialized = subscribe.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Subscribe))
      buf.getU16()

      const deserialized = Subscribe.parsePayload(buf)
      expect(deserialized).toEqual(subscribe)
      expect(buf.remaining).toBe(0)
    })

    it('should throw on partial message', () => {
      const subscribe = buildTestSubscribe()
      const serialized = subscribe.serialize()
      const serializedBytes = serialized.toUint8Array()

      const partialBytes = serializedBytes.slice(0, Math.floor(serializedBytes.length / 2))
      const buf = new ByteBuffer()
      buf.putBytes(partialBytes)

      try {
        buf.getVI()
        buf.getU16()

        expect(() => Subscribe.parsePayload(buf)).toThrow()
      } catch (err) {
        expect(err).toBeInstanceOf(Error)
      }
    })
  })
}
