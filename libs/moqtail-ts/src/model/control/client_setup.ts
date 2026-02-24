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
import { KeyValuePair } from '../common/pair'
import { LengthExceedsMaxError } from '../error/error'

export class ClientSetup {
  constructor(public readonly setupParameters: KeyValuePair[]) {}

  getType(): ControlMessageType {
    return ControlMessageType.ClientSetup
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.ClientSetup)
    const payload = new ByteBuffer()
    payload.putVI(this.setupParameters.length)
    for (const param of this.setupParameters) {
      payload.putKeyValuePair(param)
    }
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('ClientSetup::serialize(payload_length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): ClientSetup {
    const paramCount = buf.getNumberVI()
    const setupParameters: KeyValuePair[] = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      setupParameters[i] = buf.getKeyValuePair()
    }
    return new ClientSetup(setupParameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('ClientSetup', () => {
    test('roundtrip', () => {
      const setupParameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const clientSetup = new ClientSetup(setupParameters)
      const frozen = clientSetup.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.ClientSetup))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = ClientSetup.parsePayload(frozen)
      expect(deserialized.setupParameters).toEqual(setupParameters)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const setupParameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const clientSetup = new ClientSetup(setupParameters)
      const serialized = clientSetup.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.ClientSetup))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = ClientSetup.parsePayload(frozen)
      expect(deserialized.setupParameters).toEqual(setupParameters)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const setupParameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const clientSetup = new ClientSetup(setupParameters)
      const serialized = clientSetup.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const buf = new FrozenByteBuffer(partial)
      expect(() => {
        buf.getVI()
        buf.getU16()
        ClientSetup.parsePayload(buf)
      }).toThrow()
    })
  })
}
