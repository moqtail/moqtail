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

import { ControlMessageType, GroupOrder, LocationFilterType } from './constant'
import { FullTrackName } from '../data'
import {
  MessageParameter,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { LocationFilter } from '../parameter/message/location_filter'
import { Forward } from '../parameter/message/forward'
import { GroupOrderParam } from '../parameter/message/group_order_param'

export class Subscribe {
  constructor(
    public requestId: bigint,
    public fullTrackName: FullTrackName,
    public parameters: MessageParameter[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.Subscribe
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Subscribe)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putBytes(this.fullTrackName.serialize().toUint8Array())

    payload.putVI(this.parameters.length)
    payload.putBytes(serializeMessageParameterKvps(this.parameters.map((p) => p.toKeyValuePair())).toUint8Array())

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
    for (const kvp of deserializeMessageParameterKvps(buf, paramCount)) {
      const param = MessageParameter.fromKeyValuePair(kvp)
      if (param !== undefined) parameters.push(param)
    }

    return new Subscribe(requestId, fullTrackName, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  function buildTestSubscribe(): Subscribe {
    const subscribe = new Subscribe(128242n, FullTrackName.tryNew('track/namespace', 'trackName'), [
      new Forward(true),
      new GroupOrderParam(GroupOrder.Ascending),
      new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 81n, 81n, 100n),
    ])
    // Wire encoding canonicalizes parameter order ascending by type (delta-encoding requirement).
    subscribe.parameters.sort((a, b) => {
      const at = a.toKeyValuePair().typeValue
      const bt = b.toKeyValuePair().typeValue
      return at < bt ? -1 : at > bt ? 1 : 0
    })
    return subscribe
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

    it('carries its Location Filter in parameters', () => {
      const subscribe = buildTestSubscribe()
      const filter = subscribe.parameters.find((p): p is LocationFilter => p instanceof LocationFilter)
      expect(filter?.filterType).toBe(LocationFilterType.AbsoluteStartGroupEnd)
      expect(filter?.startLocation()).toEqual(new Location(81n, 81n))
      expect(filter?.endGroup).toBe(100n)
    })

    it('should handle empty parameters', () => {
      const subscribe = new Subscribe(128242n, FullTrackName.tryNew('track/namespace', 'trackName'), [])

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
