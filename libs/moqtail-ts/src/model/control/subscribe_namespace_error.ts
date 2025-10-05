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
import { ReasonPhrase } from '../common/reason_phrase'
import { ControlMessageType, SubscribeNamespaceErrorCode, subscribeNamespaceErrorCodeFromBigInt } from './constant'
import { LengthExceedsMaxError } from '../error/error'

export class SubscribeNamespaceError {
  constructor(
    public readonly requestId: bigint,
    public readonly errorCode: SubscribeNamespaceErrorCode,
    public readonly reasonPhrase: ReasonPhrase,
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.SubscribeNamespaceError
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.SubscribeNamespaceError)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.errorCode)
    payload.putReasonPhrase(this.reasonPhrase)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('SubscribeNamespaceError::serialize(payload_length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): SubscribeNamespaceError {
    const requestId = buf.getVI()
    const errorCodeRaw = buf.getVI()
    const errorCode = subscribeNamespaceErrorCodeFromBigInt(errorCodeRaw)
    const reasonPhrase = buf.getReasonPhrase()
    return new SubscribeNamespaceError(requestId, errorCode, reasonPhrase)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('SubscribeNamespaceError', () => {
    test('roundtrip', () => {
      const requestId = 662607n
      const errorCode = SubscribeNamespaceErrorCode.ExpiredAuthToken
      const reasonPhrase = new ReasonPhrase('Cheap weiners on aisle 9')
      const msg = new SubscribeNamespaceError(requestId, errorCode, reasonPhrase)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeNamespaceError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = SubscribeNamespaceError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.errorCode).toBe(msg.errorCode)
      expect(deserialized.reasonPhrase.phrase).toBe(msg.reasonPhrase.phrase)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const requestId = 662607n
      const errorCode = SubscribeNamespaceErrorCode.ExpiredAuthToken
      const reasonPhrase = new ReasonPhrase('Cheap weiners on aisle 9')
      const msg = new SubscribeNamespaceError(requestId, errorCode, reasonPhrase)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeNamespaceError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = SubscribeNamespaceError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.errorCode).toBe(msg.errorCode)
      expect(deserialized.reasonPhrase.phrase).toBe(msg.reasonPhrase.phrase)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const requestId = 662607n
      const errorCode = SubscribeNamespaceErrorCode.ExpiredAuthToken
      const reasonPhrase = new ReasonPhrase('Cheap weiners on aisle 9')
      const msg = new SubscribeNamespaceError(requestId, errorCode, reasonPhrase)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        SubscribeNamespaceError.parsePayload(frozen)
      }).toThrow()
    })
  })
}
