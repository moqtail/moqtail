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
import { ControlMessageType, FilterType } from './constant'
import { CastingError } from '../error/error'
import { MessageParameter } from '../parameter/message_parameter'
import { SubscriptionFilter } from '../parameter/message/subscription_filter'
import { Forward } from '../parameter/message/forward'

export class RequestUpdate {
  constructor(
    public requestId: bigint,
    public existingRequestId: bigint,
    public parameters: MessageParameter[],
  ) {}

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.RequestUpdate)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.existingRequestId)
    payload.putVI(this.parameters.length)

    for (const param of this.parameters) {
      payload.putBytes(param.toKeyValuePair().serialize().toUint8Array())
    }

    const payloadBytes = payload.toUint8Array()
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): RequestUpdate {
    const requestId = buf.getVI()
    const existingRequestId = buf.getVI()

    const paramCountBig = buf.getVI()
    const paramCount = Number(paramCountBig)
    if (BigInt(paramCount) !== paramCountBig) {
      throw new CastingError('RequestUpdate.deserialize paramCount', 'bigint', 'number', `${paramCountBig}`)
    }

    const parameters: MessageParameter[] = []
    for (let i = 0; i < paramCount; i++) {
      const kvp = KeyValuePair.deserialize(buf)
      const param = MessageParameter.fromKeyValuePair(kvp)
      if (param !== undefined) parameters.push(param)
    }

    return new RequestUpdate(requestId, existingRequestId, parameters)
  }

  equals(other: RequestUpdate): boolean {
    if (
      this.requestId !== other.requestId ||
      this.existingRequestId !== other.existingRequestId ||
      this.parameters.length !== other.parameters.length
    ) {
      return false
    }

    for (let i = 0; i < this.parameters.length; i++) {
      const a = this.parameters[i]
      const b = other.parameters[i]
      if (!a || !b) return false
      const aKvp = a.toKeyValuePair()
      const bKvp = b.toKeyValuePair()
      if (!aKvp.equals(bKvp)) return false
    }

    return true
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  describe('RequestUpdate', () => {
    function buildTestUpdate(): RequestUpdate {
      return new RequestUpdate(120205n, 120204n, [
        new SubscriptionFilter(FilterType.AbsoluteRange, new Location(81n, 81n), 25n),
        new Forward(true),
      ])
    }

    it('should roundtrip correctly', () => {
      const update = buildTestUpdate()
      const serialized = update.serialize()

      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestUpdate))

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining)

      const deserialized = RequestUpdate.parsePayload(buf)
      expect(deserialized.equals(update)).toBe(true)
      expect(buf.remaining).toBe(0)
    })

    it('should roundtrip with excess trailing bytes', () => {
      const update = buildTestUpdate()
      const serialized = update.serialize()
      const extra = new Uint8Array([...serialized.toUint8Array(), 9, 1, 1])

      const buf = new ByteBuffer()
      buf.putBytes(extra)

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestUpdate))

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining - 3)

      const deserialized = RequestUpdate.parsePayload(buf)
      expect(deserialized.equals(update)).toBe(true)

      const trailing = buf.toUint8Array().slice(buf.offset)
      expect(Array.from(trailing)).toEqual([9, 1, 1])
    })

    it('should throw on partial message', () => {
      const update = buildTestUpdate()
      const serialized = update.serialize()
      const serializedBytes = serialized.toUint8Array()
      const partial = serializedBytes.slice(0, Math.floor(serializedBytes.length / 2))

      const buf = new ByteBuffer()
      buf.putBytes(partial)

      try {
        buf.getVI()
        buf.getU16()
        expect(() => RequestUpdate.parsePayload(buf)).toThrow()
      } catch (err) {
        expect(err).toBeInstanceOf(Error)
      }
    })

    it('should handle empty parameters', () => {
      const update = new RequestUpdate(120206n, 120205n, [])
      const serialized = update.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      buf.getVI()
      buf.getU16()
      const deserialized = RequestUpdate.parsePayload(buf)
      expect(deserialized.equals(update)).toBe(true)
    })
  })
}
