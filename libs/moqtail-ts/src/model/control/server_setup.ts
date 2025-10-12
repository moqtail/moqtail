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
import { ControlMessageType, DRAFT_14 } from './constant'
import { KeyValuePair } from '../common/pair'
import { LengthExceedsMaxError } from '../error/error'

export class ServerSetup {
  constructor(
    public readonly selectedVersion: number,
    public readonly setupParameters: KeyValuePair[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.ServerSetup
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.ServerSetup)
    const payload = new ByteBuffer()
    payload.putVI(this.selectedVersion)
    payload.putVI(this.setupParameters.length)
    for (const param of this.setupParameters) {
      payload.putKeyValuePair(param)
    }
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('ServerSetup::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): ServerSetup {
    const selectedVersion = buf.getNumberVI()
    const paramCount = buf.getNumberVI()
    const setupParameters: KeyValuePair[] = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      setupParameters[i] = buf.getKeyValuePair()
    }
    return new ServerSetup(selectedVersion, setupParameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('ServerSetup', () => {
    test('roundtrip serialization/deserialization', () => {
      const selectedVersion = DRAFT_14
      const setupParameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const msg = new ServerSetup(selectedVersion, setupParameters)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.ServerSetup))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const parsed = ServerSetup.parsePayload(frozen)
      expect(parsed.selectedVersion).toBe(selectedVersion)
      expect(parsed.setupParameters).toEqual(setupParameters)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const selectedVersion = DRAFT_14
      const setupParameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const msg = new ServerSetup(selectedVersion, setupParameters)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.ServerSetup))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const parsed = ServerSetup.parsePayload(frozen)
      expect(parsed.selectedVersion).toBe(selectedVersion)
      expect(parsed.setupParameters).toEqual(setupParameters)
      expect(frozen.remaining).toBe(3)
    })

    test('partial message fails', () => {
      const selectedVersion = DRAFT_14
      const setupParameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const msg = new ServerSetup(selectedVersion, setupParameters)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        ServerSetup.parsePayload(frozen)
      }).toThrow()
    })
  })
}
