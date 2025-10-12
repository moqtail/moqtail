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
import { ControlMessageType, GroupOrder, groupOrderFromNumber } from './constant'
import { LengthExceedsMaxError, NotEnoughBytesError, ProtocolViolationError } from '../error/error'

export class SubscribeOk {
  requestId: bigint
  trackAlias: bigint
  expires: bigint
  groupOrder: GroupOrder
  contentExists: boolean
  largestLocation?: Location | undefined
  parameters: KeyValuePair[]

  private constructor(
    requestId: bigint,
    trackAlias: bigint,
    expires: bigint,
    groupOrder: GroupOrder,
    contentExists: boolean,
    largestLocation: Location | undefined,
    parameters: KeyValuePair[],
  ) {
    this.requestId = requestId
    this.trackAlias = trackAlias
    this.expires = expires
    this.groupOrder = groupOrder
    this.contentExists = contentExists
    this.largestLocation = largestLocation
    this.parameters = parameters
  }

  static newAscendingNoContent(
    requestId: bigint,
    trackAlias: bigint,
    expires: bigint,
    parameters: KeyValuePair[],
  ): SubscribeOk {
    return new SubscribeOk(requestId, trackAlias, expires, GroupOrder.Ascending, false, undefined, parameters)
  }

  static newDescendingNoContent(
    requestId: bigint,
    trackAlias: bigint,
    expires: bigint,
    parameters: KeyValuePair[],
  ): SubscribeOk {
    return new SubscribeOk(requestId, trackAlias, expires, GroupOrder.Descending, false, undefined, parameters)
  }

  static newAscendingWithContent(
    requestId: bigint,
    trackAlias: bigint,
    expires: bigint,
    largestLocation: Location,
    parameters: KeyValuePair[],
  ): SubscribeOk {
    return new SubscribeOk(requestId, trackAlias, expires, GroupOrder.Ascending, true, largestLocation, parameters)
  }

  static newDescendingWithContent(
    requestId: bigint,
    trackAlias: bigint,
    expires: bigint,
    largestLocation: Location,
    parameters: KeyValuePair[],
  ): SubscribeOk {
    return new SubscribeOk(requestId, trackAlias, expires, GroupOrder.Descending, true, largestLocation, parameters)
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.SubscribeOk)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.trackAlias)
    payload.putVI(this.expires)
    payload.putU8(this.groupOrder)
    if (this.contentExists) {
      payload.putU8(1)
      payload.putLocation(this.largestLocation!)
    } else {
      payload.putU8(0)
    }
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param)
    }
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
    const expires = buf.getVI()
    if (buf.remaining < 1) throw new NotEnoughBytesError('SubscribeOk::parsePayload(groupOrder)', 1, buf.remaining)
    const groupOrderRaw = buf.getU8()
    const groupOrder = groupOrderFromNumber(groupOrderRaw)
    if (groupOrder === GroupOrder.Original) {
      throw new ProtocolViolationError(
        'SubscribeOk::parsePayload(groupOrder)',
        'Group order must be Ascending(0x01) or Descending(0x02)',
      )
    }
    if (buf.remaining < 1) throw new NotEnoughBytesError('SubscribeOk::parsePayload(contentExists)', 1, buf.remaining)
    const contentExistsRaw = buf.getU8()
    let contentExists: boolean
    if (contentExistsRaw === 0) {
      contentExists = false
    } else if (contentExistsRaw === 1) {
      contentExists = true
    } else {
      throw new ProtocolViolationError('SubscribeOk::parsePayload', `Invalid Content Exists value: ${contentExistsRaw}`)
    }
    let largestLocation: Location | undefined = undefined
    if (contentExists) {
      largestLocation = buf.getLocation()
    }
    const paramCount = buf.getNumberVI()
    const parameters: KeyValuePair[] = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      parameters[i] = buf.getKeyValuePair()
    }
    return new SubscribeOk(requestId, trackAlias, expires, groupOrder, contentExists, largestLocation, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('SubscribeOk', () => {
    test('roundtrip', () => {
      const requestId = 145136n
      const trackAlias = 999n
      const expires = 16n
      const largestLocation = new Location(34n, 0n)
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('9 gifted subs from Dr.Doofishtein')),
      ]
      const subscribeOk = SubscribeOk.newAscendingWithContent(
        requestId,
        trackAlias,
        expires,
        largestLocation,
        parameters,
      )
      const frozen = subscribeOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = SubscribeOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(subscribeOk.requestId)
      expect(deserialized.trackAlias).toBe(subscribeOk.trackAlias)
      expect(deserialized.expires).toBe(subscribeOk.expires)
      expect(deserialized.groupOrder).toBe(subscribeOk.groupOrder)
      expect(deserialized.contentExists).toBe(subscribeOk.contentExists)
      expect(deserialized.largestLocation?.equals(largestLocation)).toBe(true)
      expect(deserialized.parameters).toEqual(subscribeOk.parameters)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 145136n
      const trackAlias = 999n
      const expires = 16n
      const largestLocation = new Location(34n, 0n)
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('9 gifted subs from Dr.Doofishtein')),
      ]
      const subscribeOk = SubscribeOk.newAscendingWithContent(
        requestId,
        trackAlias,
        expires,
        largestLocation,
        parameters,
      )
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
      const deserialized = SubscribeOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(subscribeOk.requestId)
      expect(deserialized.trackAlias).toBe(subscribeOk.trackAlias)
      expect(deserialized.expires).toBe(subscribeOk.expires)
      expect(deserialized.groupOrder).toBe(subscribeOk.groupOrder)
      expect(deserialized.contentExists).toBe(subscribeOk.contentExists)
      expect(deserialized.largestLocation?.equals(largestLocation)).toBe(true)
      expect(deserialized.parameters).toEqual(subscribeOk.parameters)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 145136n
      const trackAlias = 999n
      const expires = 16n

      const largestLocation = new Location(34n, 0n)
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('9 gifted subs from Dr.Doofishtein')),
      ]
      const subscribeOk = SubscribeOk.newAscendingWithContent(
        requestId,
        trackAlias,
        expires,
        largestLocation,
        parameters,
      )
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
