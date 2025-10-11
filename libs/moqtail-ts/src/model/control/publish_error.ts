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
import { LengthExceedsMaxError } from '../error/error'
import { ControlMessageType, PublishErrorCode, publishErrorCodeFromBigInt } from './constant'

export class PublishError {
  constructor(
    public readonly requestId: bigint,
    public readonly errorCode: PublishErrorCode,
    public readonly errorReason: ReasonPhrase,
  ) {}

  static new(requestId: bigint | number, errorCode: PublishErrorCode, errorReason: ReasonPhrase): PublishError {
    return new PublishError(BigInt(requestId), errorCode, errorReason)
  }

  getType(): ControlMessageType {
    return ControlMessageType.PublishError
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishError)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.errorCode)
    payload.putReasonPhrase(this.errorReason)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishError::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): PublishError {
    const requestId = buf.getVI()
    const errorCodeRaw = buf.getVI()
    const errorCode = publishErrorCodeFromBigInt(errorCodeRaw)
    const errorReason = buf.getReasonPhrase()
    return new PublishError(requestId, errorCode, errorReason)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('PublishError', () => {
    test('roundtrip', () => {
      const requestId = 12345n
      const errorCode = PublishErrorCode.Unauthorized
      const errorReason = new ReasonPhrase('Access denied for this namespace')
      const publishError = PublishError.new(requestId, errorCode, errorReason)

      const frozen = publishError.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishError.requestId)
      expect(deserialized.errorCode).toBe(publishError.errorCode)
      expect(deserialized.errorReason.phrase).toBe(publishError.errorReason.phrase)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with timeout error', () => {
      const requestId = 67890n
      const errorCode = PublishErrorCode.Timeout
      const errorReason = new ReasonPhrase('Request timed out')
      const publishError = PublishError.new(requestId, errorCode, errorReason)

      const frozen = publishError.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishError.requestId)
      expect(deserialized.errorCode).toBe(publishError.errorCode)
      expect(deserialized.errorReason.phrase).toBe(publishError.errorReason.phrase)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 12345n
      const errorCode = PublishErrorCode.InvalidNamespace
      const errorReason = new ReasonPhrase('Namespace format is invalid')
      const publishError = PublishError.new(requestId, errorCode, errorReason)
      const serialized = publishError.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishError.requestId)
      expect(deserialized.errorCode).toBe(publishError.errorCode)
      expect(deserialized.errorReason.phrase).toBe(publishError.errorReason.phrase)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 12345n
      const errorCode = PublishErrorCode.InternalError
      const errorReason = new ReasonPhrase('Internal server error occurred')
      const publishError = PublishError.new(requestId, errorCode, errorReason)
      const serialized = publishError.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        PublishError.parsePayload(frozen)
      }).toThrow()
    })
  })
}
