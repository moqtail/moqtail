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
import { KeyValuePair } from '../common/pair'
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'

export class SubscribeNamespace {
  constructor(
    public readonly requestId: bigint,
    public readonly trackNamespacePrefix: Tuple,
    public readonly parameters: KeyValuePair[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.SubscribeNamespace
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.SubscribeNamespace)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putTuple(this.trackNamespacePrefix)
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param)
    }
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('SubscribeNamespace::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): SubscribeNamespace {
    const requestId = buf.getVI()
    const trackNamespacePrefix = buf.getTuple()
    const paramCount = buf.getNumberVI()
    const parameters: KeyValuePair[] = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      parameters[i] = buf.getKeyValuePair()
    }
    return new SubscribeNamespace(requestId, trackNamespacePrefix, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('SubscribeNamespace', () => {
    test('roundtrip', () => {
      const requestId = 241421n
      const trackNamespacePrefix = Tuple.fromUtf8Path('pre/fix/me')
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Roggan?!')),
      ]
      const msg = new SubscribeNamespace(requestId, trackNamespacePrefix, parameters)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeNamespace))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = SubscribeNamespace.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.trackNamespacePrefix.equals(msg.trackNamespacePrefix)).toBe(true)
      expect(deserialized.parameters).toEqual(msg.parameters)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const requestId = 241421n
      const trackNamespacePrefix = Tuple.fromUtf8Path('pre/fix/me')
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Roggan?!')),
      ]
      const msg = new SubscribeNamespace(requestId, trackNamespacePrefix, parameters)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeNamespace))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = SubscribeNamespace.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.trackNamespacePrefix.equals(msg.trackNamespacePrefix)).toBe(true)
      expect(deserialized.parameters).toEqual(msg.parameters)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const requestId = 241421n
      const trackNamespacePrefix = Tuple.fromUtf8Path('pre/fix/me')
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Roggan?!')),
      ]
      const msg = new SubscribeNamespace(requestId, trackNamespacePrefix, parameters)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        SubscribeNamespace.parsePayload(frozen)
      }).toThrow()
    })
  })
}
