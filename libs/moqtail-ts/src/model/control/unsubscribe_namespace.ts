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

import { ByteBuffer, FrozenByteBuffer, BaseByteBuffer } from '../common/byte_buffer'
import { ControlMessageType } from './constant'
import { Tuple } from '../common/tuple'
import { LengthExceedsMaxError } from '../error/error'

export class UnsubscribeNamespace {
  constructor(public readonly trackNamespacePrefix: Tuple) {}

  getType(): ControlMessageType {
    return ControlMessageType.UnsubscribeNamespace
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.UnsubscribeNamespace)
    const payload = new ByteBuffer()
    payload.putTuple(this.trackNamespacePrefix)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('UnsubscribeNamespace::serialize(payload_length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): UnsubscribeNamespace {
    const trackNamespacePrefix = buf.getTuple()
    return new UnsubscribeNamespace(trackNamespacePrefix)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('UnsubscribeNamespace', () => {
    test('roundtrip', () => {
      const trackNamespacePrefix = Tuple.fromUtf8Path('un/announce/me')
      const unsubscribeNamespace = new UnsubscribeNamespace(trackNamespacePrefix)
      const frozen = unsubscribeNamespace.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.UnsubscribeNamespace))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = UnsubscribeNamespace.parsePayload(frozen)
      expect(deserialized.trackNamespacePrefix.equals(trackNamespacePrefix)).toBe(true)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const trackNamespacePrefix = Tuple.fromUtf8Path('un/announce/me')
      const unsubscribeNamespace = new UnsubscribeNamespace(trackNamespacePrefix)
      const serialized = unsubscribeNamespace.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer(excess.length)
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.UnsubscribeNamespace))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = UnsubscribeNamespace.parsePayload(frozen)
      expect(deserialized.trackNamespacePrefix).toEqual(trackNamespacePrefix)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const trackNamespacePrefix = Tuple.fromUtf8Path('un/announce/me')
      const unsubscribeNamespace = new UnsubscribeNamespace(trackNamespacePrefix)
      const serialized = unsubscribeNamespace.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        UnsubscribeNamespace.parsePayload(frozen)
      }).toThrow()
    })
  })
}
