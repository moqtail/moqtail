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
import { ControlMessageType, SubscribeErrorCode, subscribeErrorCodeFromBigInt } from './constant'

export class SubscribeError {
  constructor(
    public readonly requestId: bigint,
    public readonly errorCode: SubscribeErrorCode,
    public readonly errorReason: ReasonPhrase,
    public readonly trackAlias: bigint,
  ) {}

  static new(
    requestId: bigint | number,
    errorCode: SubscribeErrorCode,
    errorReason: ReasonPhrase,
    trackAlias: bigint,
  ): SubscribeError {
    return new SubscribeError(BigInt(requestId), errorCode, errorReason, trackAlias)
  }

  getType(): ControlMessageType {
    return ControlMessageType.SubscribeError
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.SubscribeError)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.errorCode)
    payload.putReasonPhrase(this.errorReason)
    payload.putVI(this.trackAlias)
    const payloadBytes = payload.toUint8Array()

    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('SubscribeError::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): SubscribeError {
    const requestId = buf.getVI()
    const errorCodeRaw = buf.getVI()
    const errorCode = subscribeErrorCodeFromBigInt(errorCodeRaw)
    const errorReason = buf.getReasonPhrase()
    const trackAlias = buf.getVI()
    return new SubscribeError(requestId, errorCode, errorReason, trackAlias)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SubscribeError', () => {
    test('roundtrip', () => {
      const requestId = 12345n
      const errorCode = SubscribeErrorCode.InvalidRange
      const errorReason = new ReasonPhrase('Lorem ipsum dolor sit amet')
      const trackAlias = 123n
      const subscribeError = SubscribeError.new(requestId, errorCode, errorReason, trackAlias)
      const frozen = subscribeError.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = SubscribeError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(subscribeError.requestId)
      expect(deserialized.errorCode).toBe(subscribeError.errorCode)
      expect(deserialized.errorReason.phrase).toBe(subscribeError.errorReason.phrase)
      expect(deserialized.trackAlias).toBe(subscribeError.trackAlias)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 12345n
      const errorCode = SubscribeErrorCode.InvalidRange
      const errorReason = new ReasonPhrase('Lorem ipsum dolor sit amet')
      const trackAlias = 123n
      const subscribeError = SubscribeError.new(requestId, errorCode, errorReason, trackAlias)

      const serialized = subscribeError.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = SubscribeError.parsePayload(frozen)
      expect(deserialized.requestId).toBe(subscribeError.requestId)
      expect(deserialized.errorCode).toBe(subscribeError.errorCode)
      expect(deserialized.errorReason.phrase).toBe(subscribeError.errorReason.phrase)
      expect(deserialized.trackAlias).toBe(subscribeError.trackAlias)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 12345n
      const errorCode = SubscribeErrorCode.InvalidRange
      const errorReason = new ReasonPhrase('Lorem ipsum dolor sit amet')
      const trackAlias = 123n
      const subscribeError = SubscribeError.new(requestId, errorCode, errorReason, trackAlias)
      const serialized = subscribeError.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        SubscribeError.parsePayload(frozen)
      }).toThrow()
    })
  })
}
