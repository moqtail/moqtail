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
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'

export class PublishNamespaceOk {
  public readonly requestId: bigint

  constructor(requestId: bigint | number) {
    this.requestId = BigInt(requestId)
  }

  getType(): ControlMessageType {
    return ControlMessageType.PublishNamespaceOk
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    buf.putVI(ControlMessageType.PublishNamespaceOk)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishNamespaceOk::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): PublishNamespaceOk {
    const requestId = buf.getVI()
    return new PublishNamespaceOk(requestId)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('PublishNamespaceOk', () => {
    test('roundtrip', () => {
      const requestId = 12345n
      const announceOk = new PublishNamespaceOk(requestId)
      const frozen = announceOk.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishNamespaceOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishNamespaceOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(announceOk.requestId)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const requestId = 67890n
      const announceOk = new PublishNamespaceOk(requestId)
      const serialized = announceOk.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishNamespaceOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishNamespaceOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(announceOk.requestId)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const requestId = 112233n
      const announceOk = new PublishNamespaceOk(requestId)
      const serialized = announceOk.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        PublishNamespaceOk.parsePayload(frozen)
      }).toThrow()
    })
  })
}
