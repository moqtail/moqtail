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
import { Tuple } from '../common/tuple'
import { ControlMessageType, GroupOrder, LocationFilterType } from './constant'
import { FullTrackName } from '../data'
import {
  MessageParameter,
  MessageParameters,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { AuthorizationToken } from '../parameter/common/authorization_token'
import { LocationFilter } from '../parameter/message/location_filter'

export class TrackStatus {
  constructor(
    public requestId: bigint,
    public trackAlias: bigint,
    public fullTrackName: FullTrackName,
    public subscriberPriority: number,
    public groupOrder: GroupOrder,
    public forward: boolean,
    public locationFilter: LocationFilter,
    public subscribeParameters: MessageParameter[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.TrackStatus
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.TrackStatus)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.trackAlias)
    payload.putBytes(this.fullTrackName.serialize().toUint8Array())
    payload.putU8(this.subscriberPriority)
    payload.putU8(this.groupOrder)
    payload.putU8(this.forward ? 1 : 0)
    payload.putBytes(this.locationFilter.serializeValue())

    payload.putVI(this.subscribeParameters.length)
    payload.putBytes(
      serializeMessageParameterKvps(this.subscribeParameters.map((p) => p.toKeyValuePair())).toUint8Array(),
    )

    const payloadBytes = payload.toUint8Array()
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): TrackStatus {
    const requestId = buf.getVI()
    const trackAlias = buf.getVI()
    const fullTrackName = buf.getFullTrackName()
    const subscriberPriority = buf.getU8()
    const groupOrder = buf.getU8()
    const forward = buf.getU8() === 1
    const locationFilter = LocationFilter.deserializeValue(buf)

    const paramCount = Number(buf.getVI())
    const rawParams = deserializeMessageParameterKvps(buf, paramCount)
    const subscribeParameters = MessageParameters.fromKeyValuePairs(rawParams)

    return new TrackStatus(
      requestId,
      trackAlias,
      fullTrackName,
      subscriberPriority,
      groupOrder,
      forward,
      locationFilter,
      subscribeParameters,
    )
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  function buildTestTrackStatus(): TrackStatus {
    return new TrackStatus(
      128242n,
      999n,
      FullTrackName.tryNew('track/namespace', 'trackName'),
      31,
      GroupOrder.Original,
      true,
      new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 81n, 81n, 100n),
      [AuthorizationToken.newUseValue(0n, new TextEncoder().encode('test-token'))],
    )
  }

  describe('TrackStatus', () => {
    it('should roundtrip correctly', () => {
      const subscribe = buildTestTrackStatus()
      const serialized = subscribe.serialize()

      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.TrackStatus))

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining)

      const deserialized = TrackStatus.parsePayload(buf)
      expect(deserialized).toEqual(subscribe)
      expect(buf.remaining).toBe(0)
    })

    it('should roundtrip with excess trailing bytes', () => {
      const subscribe = buildTestTrackStatus()
      const serialized = subscribe.serialize()
      const extra = new Uint8Array([...serialized.toUint8Array(), 9, 1, 1])

      const buf = new ByteBuffer()
      buf.putBytes(extra)

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.TrackStatus))

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining - 3)

      const deserialized = TrackStatus.parsePayload(buf)
      expect(deserialized).toEqual(subscribe)

      const trailing = buf.toUint8Array().slice(buf.offset)
      expect(Array.from(trailing)).toEqual([9, 1, 1])
    })

    it('should throw on invalid filterType', () => {
      const buf = new ByteBuffer()
      buf.putVI(ControlMessageType.TrackStatus)
      buf.putVI(128242n)
      buf.putVI(999n)
      buf.putTuple(Tuple.fromUtf8Path('invalid/filter'))
      buf.putVI(10)
      buf.putBytes(new TextEncoder().encode('InvalidTest'))
      buf.putU8(31)
      buf.putU8(GroupOrder.Original)
      buf.putU8(1)
      buf.putVI(9999)

      expect(() => TrackStatus.parsePayload(buf)).toThrow()
    })
    it('should handle empty subscribeParameters', () => {
      const subscribe = new TrackStatus(
        128242n,
        999n,
        FullTrackName.tryNew('track/namespace', 'trackName'),
        31,
        GroupOrder.Original,
        true,
        new LocationFilter(LocationFilterType.NextObject),
        [],
      )

      const serialized = subscribe.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.TrackStatus))
      buf.getU16()

      const deserialized = TrackStatus.parsePayload(buf)
      expect(deserialized).toEqual(subscribe)
      expect(buf.remaining).toBe(0)
    })

    it('should throw on partial message', () => {
      const subscribe = buildTestTrackStatus()
      const serialized = subscribe.serialize()
      const serializedBytes = serialized.toUint8Array()

      const partialBytes = serializedBytes.slice(0, Math.floor(serializedBytes.length / 2))
      const buf = new ByteBuffer()
      buf.putBytes(partialBytes)

      try {
        buf.getVI()
        buf.getU16()

        expect(() => TrackStatus.parsePayload(buf)).toThrow()
      } catch (err) {
        expect(err).toBeInstanceOf(Error)
      }
    })
  })
}
