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
import { Tuple } from '../common/tuple'
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'

/**
 * @public
 * Represents a protocol PublishNamespaceDone message, used to unpublish a track namespace.
 */
export class PublishNamespaceDone {
  /**
   * Constructs a PublishNamespaceDone message.
   * @param trackNamespace - The track namespace to unpublish.
   */
  constructor(public readonly trackNamespace: Tuple) {}

  /**
   * Gets the control message type for this PublishNamespaceDone message.
   * @returns The ControlMessageType.PublishNamespaceDone enum value.
   */
  getType(): ControlMessageType {
    return ControlMessageType.PublishNamespaceDone
  }

  /**
   * Serializes the PublishNamespaceDone message to a frozen byte buffer.
   * @returns The serialized message as a FrozenByteBuffer.
   * @throws :{@link LengthExceedsMaxError} If the payload exceeds 65535 bytes.
   */
  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishNamespaceDone)
    const payload = new ByteBuffer()
    payload.putTuple(this.trackNamespace)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError(
        'PublishNamespaceDone::serialize(payloadBytes.length)',
        0xffff,
        payloadBytes.length,
      )
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  /**
   * Parses a PublishNamespaceDone message payload from a buffer.
   * @param buf - The buffer containing the payload.
   * @returns The parsed PublishNamespaceDone message.
   */
  static parsePayload(buf: BaseByteBuffer): PublishNamespaceDone {
    const trackNamespace = buf.getTuple()
    return new PublishNamespaceDone(trackNamespace)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('PublishNamespaceDone', () => {
    test('roundtrip', () => {
      const trackNamespace = Tuple.fromUtf8Path('un/announce/me')
      const msg = new PublishNamespaceDone(trackNamespace)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishNamespaceDone))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishNamespaceDone.parsePayload(frozen)
      expect(deserialized.trackNamespace.equals(msg.trackNamespace)).toBe(true)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const trackNamespace = Tuple.fromUtf8Path('un/announce/me')
      const msg = new PublishNamespaceDone(trackNamespace)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishNamespaceDone))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishNamespaceDone.parsePayload(frozen)
      expect(deserialized.trackNamespace.equals(msg.trackNamespace)).toBe(true)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const trackNamespace = Tuple.fromUtf8Path('un/announce/me')
      const msg = new PublishNamespaceDone(trackNamespace)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const buf = new ByteBuffer()
      buf.putBytes(partial)
      const frozen = buf.freeze()
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        PublishNamespaceDone.parsePayload(frozen)
      }).toThrow()
    })
  })
}
