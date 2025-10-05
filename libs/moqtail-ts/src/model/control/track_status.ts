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

// TODO: Couple filter type and bounded parameters for idiomatic design
export class TrackStatus {
  private constructor(
    public requestId: bigint,
    public trackAlias: bigint,
    public fullTrackName: FullTrackName,
    public subscriberPriority: number,
    public groupOrder: GroupOrder,
    public forward: boolean,
    public filterType: FilterType,
    public startLocation: Location | undefined,
    public endGroup: bigint | undefined,
    public subscribeParameters: KeyValuePair[],
  ) {}

  static newNextGroupStart(
    requestId: bigint,
    trackAlias: bigint,
    fullTrackName: FullTrackName,
    subscriberPriority: number,
    groupOrder: GroupOrder,
    forward: boolean,
    subscribeParameters: KeyValuePair[],
  ): TrackStatus {
    return new TrackStatus(
      requestId,
      trackAlias,
      fullTrackName,
      subscriberPriority,
      groupOrder,
      forward,
      FilterType.NextGroupStart,
      undefined,
      undefined,
      subscribeParameters,
    )
  }

  static newLatestObject(
    requestId: bigint,
    trackAlias: bigint,
    fullTrackName: FullTrackName,
    subscriberPriority: number,
    groupOrder: GroupOrder,
    forward: boolean,
    subscribeParameters: KeyValuePair[],
  ): TrackStatus {
    return new TrackStatus(
      requestId,
      trackAlias,
      fullTrackName,
      subscriberPriority,
      groupOrder,
      forward,
      FilterType.LatestObject,
      undefined,
      undefined,
      subscribeParameters,
    )
  }

  static newAbsoluteStart(
    requestId: bigint,
    trackAlias: bigint,
    fullTrackName: FullTrackName,
    subscriberPriority: number,
    groupOrder: GroupOrder,
    forward: boolean,
    startLocation: Location,
    subscribeParameters: KeyValuePair[],
  ): TrackStatus {
    return new TrackStatus(
      requestId,
      trackAlias,
      fullTrackName,
      subscriberPriority,
      groupOrder,
      forward,
      FilterType.AbsoluteStart,
      startLocation,
      undefined,
      subscribeParameters,
    )
  }

  static newAbsoluteRange(
    requestId: bigint,
    trackAlias: bigint,
    fullTrackName: FullTrackName,
    subscriberPriority: number,
    groupOrder: GroupOrder,
    forward: boolean,
    startLocation: Location,
    endGroup: bigint,
    subscribeParameters: KeyValuePair[],
  ): TrackStatus {
    if (endGroup < startLocation.group) {
      throw new Error('End Group must be >= Start Group')
    }
    return new TrackStatus(
      requestId,
      trackAlias,
      fullTrackName,
      subscriberPriority,
      groupOrder,
      forward,
      FilterType.AbsoluteRange,
      startLocation,
      endGroup,
      subscribeParameters,
    )
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
    payload.putVI(this.filterType)

    if (this.filterType === FilterType.AbsoluteStart || this.filterType === FilterType.AbsoluteRange) {
      if (!this.startLocation) {
        throw new Error('StartLocation required for selected filterType')
      }
      payload.putLocation(this.startLocation)
    }

    if (this.filterType === FilterType.AbsoluteRange) {
      if (this.endGroup == null) {
        throw new Error('EndGroup required for AbsoluteRange')
      }
      payload.putVI(this.endGroup)
    }

    payload.putVI(this.subscribeParameters.length)
    for (const param of this.subscribeParameters) {
      payload.putBytes(param.serialize().toUint8Array())
    }

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
    const filterType = Number(buf.getVI()) as FilterType

    let startLocation: Location | undefined = undefined
    let endGroup: bigint | undefined = undefined

    if (filterType === FilterType.AbsoluteStart || filterType === FilterType.AbsoluteRange) {
      startLocation = buf.getLocation()
    }
    if (filterType === FilterType.AbsoluteRange) {
      endGroup = buf.getVI()
    }

    const paramCount = Number(buf.getVI())
    const subscribeParameters: KeyValuePair[] = []
    for (let i = 0; i < paramCount; i++) {
      subscribeParameters.push(KeyValuePair.deserialize(buf))
    }

    return new TrackStatus(
      requestId,
      trackAlias,
      fullTrackName,
      subscriberPriority,
      groupOrder,
      forward,
      filterType,
      startLocation,
      endGroup,
      subscribeParameters,
    )
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  function buildTestTrackStatus(): TrackStatus {
    return TrackStatus.newAbsoluteRange(
      128242n,
      999n,
      FullTrackName.tryNew('track/namespace', 'trackName'),
      31,
      GroupOrder.Original,
      true,
      new Location(81n, 81n),
      100n,
      [KeyValuePair.tryNewVarInt(0n, 10n), KeyValuePair.tryNewBytes(1n, new TextEncoder().encode('DemoString'))],
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

    describe('TrackStatus Constructors', () => {
      it('should create a TrackStatus with AbsoluteRange filter', () => {
        const subscribe = TrackStatus.newAbsoluteRange(
          128242n,
          999n,
          FullTrackName.tryNew('track/namespace', 'trackName'),
          31,
          GroupOrder.Original,
          true,
          new Location(81n, 81n),
          100n,
          [],
        )

        expect(subscribe.filterType).toBe(FilterType.AbsoluteRange)
        expect(subscribe.startLocation).toEqual(new Location(81n, 81n))
        expect(subscribe.endGroup).toBe(100n)
      })

      it('should throw an error if EndGroup < StartGroup', () => {
        expect(() =>
          TrackStatus.newAbsoluteRange(
            128242n,
            999n,
            FullTrackName.tryNew('track/namespace', 'trackName'),
            31,
            GroupOrder.Original,
            true,
            new Location(81n, 81n),
            80n,
            [],
          ),
        ).toThrow('End Group must be >= Start Group')
      })
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
      const subscribe = TrackStatus.newLatestObject(
        128242n,
        999n,
        FullTrackName.tryNew('track/namespace', 'trackName'),
        31,
        GroupOrder.Original,
        true,
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
