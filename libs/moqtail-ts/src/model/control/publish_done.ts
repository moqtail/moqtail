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
import { ControlMessageType, PublishDoneStatusCode, publishDoneStatusCodeFromBigInt } from './constant'

export class PublishDone {
  constructor(
    public readonly requestId: bigint,
    public readonly statusCode: PublishDoneStatusCode,
    public readonly streamCount: bigint,
    public readonly errorReason: ReasonPhrase,
  ) {}

  static new(
    requestId: bigint | number,
    statusCode: PublishDoneStatusCode,
    streamCount: bigint,
    errorReason: ReasonPhrase,
  ): PublishDone {
    return new PublishDone(BigInt(requestId), statusCode, streamCount, errorReason)
  }

  getType(): ControlMessageType {
    return ControlMessageType.PublishDone
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishDone)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(this.statusCode)
    payload.putVI(this.streamCount)
    payload.putReasonPhrase(this.errorReason)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishDone::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): PublishDone {
    const requestId = buf.getVI()
    const statusCodeRaw = buf.getVI()
    const statusCode = publishDoneStatusCodeFromBigInt(statusCodeRaw)
    const streamCount = buf.getVI()
    const errorReason = buf.getReasonPhrase()
    return new PublishDone(requestId, statusCode, streamCount, errorReason)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('PublishDone', () => {
    test('roundtrip', () => {
      const requestId = 12345n
      const statusCode = PublishDoneStatusCode.SubscriptionEnded
      const streamCount = 123n
      const errorReason = new ReasonPhrase('Lorem ipsum dolor sit amet')
      const publishDone = PublishDone.new(requestId, statusCode, streamCount, errorReason)

      const frozen = publishDone.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishDone))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishDone.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishDone.requestId)
      expect(deserialized.statusCode).toBe(publishDone.statusCode)
      expect(deserialized.streamCount).toBe(publishDone.streamCount)
      expect(deserialized.errorReason.phrase).toBe(publishDone.errorReason.phrase)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 12345n
      const statusCode = PublishDoneStatusCode.Expired
      const streamCount = 123n
      const errorReason = new ReasonPhrase('Lorem ipsum dolor sit amet')
      const publishDone = PublishDone.new(requestId, statusCode, streamCount, errorReason)
      const serialized = publishDone.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishDone))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishDone.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publishDone.requestId)
      expect(deserialized.statusCode).toBe(publishDone.statusCode)
      expect(deserialized.streamCount).toBe(publishDone.streamCount)
      expect(deserialized.errorReason.phrase).toBe(publishDone.errorReason.phrase)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 12345n
      const statusCode = PublishDoneStatusCode.Expired
      const streamCount = 123n
      const errorReason = new ReasonPhrase('Lorem ipsum dolor sit amet')
      const publishDone = PublishDone.new(requestId, statusCode, streamCount, errorReason)
      const serialized = publishDone.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        PublishDone.parsePayload(frozen)
      }).toThrow()
    })
  })
}
