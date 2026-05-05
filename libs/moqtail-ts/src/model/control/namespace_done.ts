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
 * Represents a NAMESPACE_DONE message sent on a SUBSCRIBE_NAMESPACE bi-stream.
 * Signals that a previously announced namespace suffix is no longer available.
 */
export class NamespaceDone {
  constructor(public readonly trackNamespaceSuffix: Tuple) {}

  getType(): ControlMessageType {
    return ControlMessageType.NamespaceDone
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.NamespaceDone)
    const payload = new ByteBuffer()
    payload.putTuple(this.trackNamespaceSuffix)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('NamespaceDone::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): NamespaceDone {
    const trackNamespaceSuffix = buf.getTuple()
    return new NamespaceDone(trackNamespaceSuffix)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('NamespaceDone', () => {
    test('roundtrip', () => {
      const suffix = Tuple.fromUtf8Path('room/stream')
      const msg = new NamespaceDone(suffix)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.NamespaceDone))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = NamespaceDone.parsePayload(frozen)
      expect(deserialized.trackNamespaceSuffix.equals(msg.trackNamespaceSuffix)).toBe(true)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const suffix = Tuple.fromUtf8Path('room/stream')
      const msg = new NamespaceDone(suffix)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.NamespaceDone))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = NamespaceDone.parsePayload(frozen)
      expect(deserialized.trackNamespaceSuffix.equals(msg.trackNamespaceSuffix)).toBe(true)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const suffix = Tuple.fromUtf8Path('room/stream')
      const msg = new NamespaceDone(suffix)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const buf = new ByteBuffer()
      buf.putBytes(partial)
      const frozen = buf.freeze()
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        NamespaceDone.parsePayload(frozen)
      }).toThrow()
    })
  })
}
