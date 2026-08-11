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
import { ControlMessageType, RequestErrorCode } from './constant'
import { LengthExceedsMaxError } from '../error/error'

/**
 * REQUEST_ERROR (0x5) refuses any request type. It carries no Request ID: the request
 * stream it arrives on identifies the request (§10.1).
 */
export class RequestError {
  public readonly errorCode: RequestErrorCode
  public readonly retryInterval: bigint
  public readonly reasonPhrase: ReasonPhrase

  constructor(errorCode: RequestErrorCode, retryInterval: bigint, reasonPhrase: ReasonPhrase) {
    this.errorCode = errorCode
    this.retryInterval = retryInterval
    this.reasonPhrase = reasonPhrase
  }

  getType(): ControlMessageType {
    return ControlMessageType.RequestError
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.RequestError)
    const payload = new ByteBuffer()
    payload.putVI(this.errorCode)
    payload.putVI(this.retryInterval)
    payload.putReasonPhrase(this.reasonPhrase)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('RequestError::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): RequestError {
    const errorCodeRaw = buf.getVI()
    const errorCode = RequestErrorCode.tryFrom(errorCodeRaw)
    const retryInterval = buf.getVI()
    const reasonPhrase = buf.getReasonPhrase()
    return new RequestError(errorCode, retryInterval, reasonPhrase)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('RequestError', () => {
    test('roundtrip', () => {
      const errorCode = RequestErrorCode.InternalError
      const retryInterval = 0n
      const reasonPhrase = new ReasonPhrase("They see me rollin'")
      const requestError = new RequestError(errorCode, retryInterval, reasonPhrase)
      const frozen = requestError.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = RequestError.parsePayload(frozen)
      expect(deserialized.errorCode).toBe(requestError.errorCode)
      expect(deserialized.retryInterval).toBe(requestError.retryInterval)
      expect(deserialized.reasonPhrase.phrase).toBe(requestError.reasonPhrase.phrase)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const errorCode = RequestErrorCode.InternalError
      const retryInterval = 0n
      const reasonPhrase = new ReasonPhrase("They see me rollin'")
      const requestError = new RequestError(errorCode, retryInterval, reasonPhrase)
      const serialized = requestError.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestError))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = RequestError.parsePayload(frozen)
      expect(deserialized.errorCode).toBe(requestError.errorCode)
      expect(deserialized.retryInterval).toBe(requestError.retryInterval)
      expect(deserialized.reasonPhrase.phrase).toBe(requestError.reasonPhrase.phrase)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const errorCode = RequestErrorCode.InternalError
      const retryInterval = 0n
      const reasonPhrase = new ReasonPhrase("They see me rollin'")
      const requestError = new RequestError(errorCode, retryInterval, reasonPhrase)
      const serialized = requestError.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        RequestError.parsePayload(frozen)
      }).toThrow()
    })
  })
}
