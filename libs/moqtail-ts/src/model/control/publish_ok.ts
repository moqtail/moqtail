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
import { KeyValuePair } from '../common/pair'
import { LengthExceedsMaxError } from '../error/error'
import { ControlMessageType, FilterType, filterTypeFromBigInt } from './constant'

export class PublishOk {
  constructor(
    public readonly requestId: bigint,
    public readonly forward: number,
    public readonly subscriberPriority: number,
    public readonly groupOrder: number,
    public readonly filterType: FilterType,
    public readonly startLocation: Location | undefined,
    public readonly endGroup: bigint | undefined,
    public readonly parameters: KeyValuePair[],
  ) {}

  static new(
    requestId: bigint | number,
    forward: number,
    subscriberPriority: number,
    groupOrder: number,
    filterType: FilterType,
    startLocation: Location | undefined,
    endGroup: bigint | number | undefined,
    parameters: KeyValuePair[],
  ): PublishOk {
    return new PublishOk(
      BigInt(requestId),
      forward,
      subscriberPriority,
      groupOrder,
      filterType,
      startLocation,
      endGroup !== undefined ? BigInt(endGroup) : undefined,
      parameters,
    )
  }

  getType(): ControlMessageType {
    return ControlMessageType.PublishOk
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishOk)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putU8(this.forward)
    payload.putU8(this.subscriberPriority)
    payload.putU8(this.groupOrder)
    payload.putVI(this.filterType)

    // Handle optional fields based on filter type
    if (this.filterType === FilterType.AbsoluteStart || this.filterType === FilterType.AbsoluteRange) {
      if (this.startLocation !== undefined) {
        payload.putBytes(this.startLocation.serialize().toUint8Array())
      }
    }
    if (this.filterType === FilterType.AbsoluteRange) {
      if (this.endGroup !== undefined) {
        payload.putVI(this.endGroup)
      }
    }

    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param)
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
    const forward = buf.getU8()
    const subscriberPriority = buf.getU8()
    const groupOrder = buf.getU8()
    const filterTypeRaw = buf.getVI()
    const filterType = filterTypeFromBigInt(filterTypeRaw)

    let startLocation: Location | undefined
    let endGroup: bigint | undefined

    // Handle optional fields based on filter type
    if (filterType === FilterType.AbsoluteStart || filterType === FilterType.AbsoluteRange) {
      startLocation = Location.deserialize(buf)
    }
    if (filterType === FilterType.AbsoluteRange) {
      endGroup = buf.getVI()
    }

    const paramCount = buf.getVI()
    const parameters: KeyValuePair[] = new Array(Number(paramCount))
    for (let i = 0; i < paramCount; i++) {
      parameters[i] = buf.getKeyValuePair()
    }

    return new PublishOk(
      requestId,
      forward,
      subscriberPriority,
      groupOrder,
      filterType,
      startLocation,
      endGroup,
      parameters,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('PublishOk', () => {
    test('roundtrip with NextGroupStart filter', () => {
      const requestId = 12345n
      const forward = 1
      const subscriberPriority = 100
      const groupOrder = 1
      const filterType = FilterType.NextGroupStart
      const startLocation = undefined
      const endGroup = undefined
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 100n),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('test')),
      ]
      const publishOk = PublishOk.new(
        requestId,
        forward,
        subscriberPriority,
        groupOrder,
        filterType,
        startLocation,
        endGroup,
        parameters,
      )

      const frozen = publishOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishOk.requestId)
      expect(deserialized.forward).toBe(publishOk.forward)
      expect(deserialized.subscriberPriority).toBe(publishOk.subscriberPriority)
      expect(deserialized.groupOrder).toBe(publishOk.groupOrder)
      expect(deserialized.filterType).toBe(publishOk.filterType)
      expect(deserialized.startLocation).toBe(undefined)
      expect(deserialized.endGroup).toBe(undefined)
      expect(deserialized.parameters.length).toBe(publishOk.parameters.length)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with AbsoluteStart filter', () => {
      const requestId = 12345n
      const forward = 1
      const subscriberPriority = 100
      const groupOrder = 1
      const filterType = FilterType.AbsoluteStart
      const startLocation = new Location(5n, 10n)
      const endGroup = undefined
      const parameters = [KeyValuePair.tryNewVarInt(0, 100n)]
      const publishOk = PublishOk.new(
        requestId,
        forward,
        subscriberPriority,
        groupOrder,
        filterType,
        startLocation,
        endGroup,
        parameters,
      )

      const frozen = publishOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishOk.requestId)
      expect(deserialized.forward).toBe(publishOk.forward)
      expect(deserialized.subscriberPriority).toBe(publishOk.subscriberPriority)
      expect(deserialized.groupOrder).toBe(publishOk.groupOrder)
      expect(deserialized.filterType).toBe(publishOk.filterType)
      expect(deserialized.startLocation?.group).toBe(startLocation.group)
      expect(deserialized.startLocation?.object).toBe(startLocation.object)
      expect(deserialized.endGroup).toBe(undefined)
      expect(deserialized.parameters.length).toBe(publishOk.parameters.length)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with AbsoluteRange filter', () => {
      const requestId = 12345n
      const forward = 1
      const subscriberPriority = 100
      const groupOrder = 1
      const filterType = FilterType.AbsoluteRange
      const startLocation = new Location(5n, 10n)
      const endGroup = 20n
      const parameters = [KeyValuePair.tryNewVarInt(0, 100n)]
      const publishOk = PublishOk.new(
        requestId,
        forward,
        subscriberPriority,
        groupOrder,
        filterType,
        startLocation,
        endGroup,
        parameters,
      )

      const frozen = publishOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishOk.requestId)
      expect(deserialized.forward).toBe(publishOk.forward)
      expect(deserialized.subscriberPriority).toBe(publishOk.subscriberPriority)
      expect(deserialized.groupOrder).toBe(publishOk.groupOrder)
      expect(deserialized.filterType).toBe(publishOk.filterType)
      expect(deserialized.startLocation?.group).toBe(startLocation.group)
      expect(deserialized.startLocation?.object).toBe(startLocation.object)
      expect(deserialized.endGroup).toBe(endGroup)
      expect(deserialized.parameters.length).toBe(publishOk.parameters.length)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 12345n
      const forward = 1
      const subscriberPriority = 100
      const groupOrder = 1
      const filterType = FilterType.LatestObject
      const startLocation = undefined
      const endGroup = undefined
      const parameters = [KeyValuePair.tryNewVarInt(0, 100n)]
      const publishOk = PublishOk.new(
        requestId,
        forward,
        subscriberPriority,
        groupOrder,
        filterType,
        startLocation,
        endGroup,
        parameters,
      )
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
      expect(deserialized.filterType).toBe(publishOk.filterType)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 12345n
      const forward = 1
      const subscriberPriority = 100
      const groupOrder = 1
      const filterType = FilterType.NextGroupStart
      const startLocation = undefined
      const endGroup = undefined
      const parameters = [KeyValuePair.tryNewVarInt(0, 100n)]
      const publishOk = PublishOk.new(
        requestId,
        forward,
        subscriberPriority,
        groupOrder,
        filterType,
        startLocation,
        endGroup,
        parameters,
      )
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
