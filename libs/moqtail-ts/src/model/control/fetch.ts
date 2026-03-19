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
import { ControlMessageType, FetchType, fetchTypeFromBigInt, GroupOrder, groupOrderFromNumber } from './constant'
import { LengthExceedsMaxError, NotEnoughBytesError } from '../error/error'
import { FullTrackName } from '../data'

export class Fetch {
  constructor(
    public readonly requestId: bigint,
    public readonly subscriberPriority: number,
    public readonly groupOrder: GroupOrder,
    public readonly typeAndProps:
      | {
          readonly type: FetchType.Standalone
          readonly props: { fullTrackName: FullTrackName; startLocation: Location; endLocation: Location }
        }
      | {
          readonly type: FetchType.Relative
          readonly props: { joiningRequestId: bigint; joiningStart: bigint }
        }
      | {
          readonly type: FetchType.Absolute
          readonly props: { joiningRequestId: bigint; joiningStart: bigint }
        },
    public readonly parameters: KeyValuePair[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.Fetch
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Fetch)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putU8(this.subscriberPriority)
    payload.putU8(this.groupOrder)
    payload.putVI(this.typeAndProps.type)
    switch (this.typeAndProps.type) {
      case FetchType.Absolute:
      case FetchType.Relative: {
        payload.putVI(this.typeAndProps.props.joiningRequestId)
        payload.putVI(this.typeAndProps.props.joiningStart)
        break
      }
      case FetchType.Standalone: {
        payload.putFullTrackName(this.typeAndProps.props.fullTrackName)
        payload.putLocation(this.typeAndProps.props.startLocation)
        payload.putLocation(this.typeAndProps.props.endLocation)
        break
      }
    }
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param)
    }
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('Fetch::serialize(payload_length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): Fetch {
    const requestId = buf.getVI()
    if (buf.remaining < 1) throw new NotEnoughBytesError('Fetch::parse_payload(subscriber_priority)', 1, buf.remaining)
    const subscriberPriority = buf.getU8()
    if (buf.remaining < 1) throw new NotEnoughBytesError('Fetch::parse_payload(group_order)', 1, buf.remaining)
    const groupOrderRaw = buf.getU8()
    const groupOrder = groupOrderFromNumber(groupOrderRaw)
    const fetchTypeRaw = buf.getVI()
    const fetchType = fetchTypeFromBigInt(fetchTypeRaw)

    let props: Fetch['typeAndProps']

    switch (fetchType) {
      case FetchType.Absolute:
      case FetchType.Relative: {
        const joiningRequestId = buf.getVI()
        const joiningStart = buf.getVI()
        props = {
          type: fetchType,
          props: { joiningRequestId, joiningStart },
        }
        break
      }
      case FetchType.Standalone: {
        const fullTrackName = buf.getFullTrackName()
        const startLocation = buf.getLocation()
        const endLocation = buf.getLocation()
        props = {
          type: FetchType.Standalone,
          props: { fullTrackName, startLocation, endLocation },
        }
        break
      }
      default:
        throw new Error(`Unknown fetch type: ${fetchType}`)
    }

    const paramCount = buf.getNumberVI()
    const parameters: KeyValuePair[] = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      parameters[i] = buf.getKeyValuePair()
    }

    return new Fetch(requestId, subscriberPriority, groupOrder, props, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('Fetch', () => {
    test('roundtrip', () => {
      const requestId = 161803n
      const subscriberPriority = 15
      const groupOrder = GroupOrder.Descending
      const parameters = [
        KeyValuePair.tryNewVarInt(4444, 12321n),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('fetch me ok')),
      ]
      const fetch = new Fetch(
        requestId,
        subscriberPriority,
        groupOrder,
        {
          type: FetchType.Absolute,
          props: { joiningRequestId: 119n, joiningStart: 73n },
        },
        parameters,
      )
      const serialized = fetch.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Fetch))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = Fetch.parsePayload(frozen)
      expect(deserialized).toEqual(fetch)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 161803n
      const subscriberPriority = 15
      const groupOrder = GroupOrder.Descending
      const parameters = [
        KeyValuePair.tryNewVarInt(4444, 12321n),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('fetch me ok')),
      ]
      const fetch = new Fetch(
        requestId,
        subscriberPriority,
        groupOrder,
        {
          type: FetchType.Absolute,
          props: { joiningRequestId: 119n, joiningStart: 73n },
        },
        parameters,
      )
      const serialized = fetch.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Fetch))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = Fetch.parsePayload(frozen)
      expect(deserialized).toEqual(fetch)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 161803n
      const subscriberPriority = 15
      const groupOrder = GroupOrder.Descending
      const parameters = [
        KeyValuePair.tryNewVarInt(4444, 12321n),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('fetch me ok')),
      ]
      const fetch = new Fetch(
        requestId,
        subscriberPriority,
        groupOrder,
        {
          type: FetchType.Absolute,
          props: { joiningRequestId: 119n, joiningStart: 73n },
        },
        parameters,
      )
      const serialized = fetch.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        Fetch.parsePayload(frozen)
      }).toThrow()
    })
  })
}
