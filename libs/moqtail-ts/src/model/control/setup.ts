/**
 * Copyright 2026 The MOQtail Authors
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
import { KeyValuePair, deserializeKvpListUntilEmpty, serializeKvpList } from '../common/pair'
import { LengthExceedsMaxError } from '../error/error'

/**
 * The first message each endpoint sends on its control stream (draft-18 §10.3).
 *
 * Both peers send the same message; there are no separate client and server forms and
 * no version fields, since version negotiation happens over ALPN (§3.1). Replaces
 * CLIENT_SETUP / SERVER_SETUP.
 *
 * Some options are client-only (see {@link SetupOptionType}); whether a peer is
 * allowed to send a given option depends on which side it is and on the transport,
 * neither of which is knowable here, so that rule is enforced by the caller rather than
 * by this class.
 */
export class Setup {
  constructor(public readonly setupOptions: KeyValuePair[]) {}

  getType(): ControlMessageType {
    return ControlMessageType.Setup
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Setup)
    // Setup Options span the entire payload, bounded by Length; there is no count (§10.3).
    const payloadBytes = serializeKvpList(this.setupOptions).toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('Setup::serialize(payload_length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): Setup {
    const setupOptions = deserializeKvpListUntilEmpty(buf)
    return new Setup(setupOptions)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Setup', () => {
    test('roundtrip', () => {
      const setupOptions = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const setup = new Setup(setupOptions)
      const frozen = setup.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Setup))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = Setup.parsePayload(frozen)
      expect(deserialized.setupOptions).toEqual(setupOptions)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with no options', () => {
      const setup = new Setup([])
      const frozen = setup.serialize()
      frozen.getVI()
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(0)
      const deserialized = Setup.parsePayload(frozen)
      expect(deserialized.setupOptions).toEqual([])
    })

    // §10.3: Setup Options are bounded by Length, not preceded by a count, unlike the
    // draft-16 CLIENT_SETUP / SERVER_SETUP payloads this replaces.
    test('payload carries no option count', () => {
      const setupOptions = [KeyValuePair.tryNewVarInt(0, 10)]
      const setup = new Setup(setupOptions)
      const frozen = setup.serialize()
      frozen.getVI()
      frozen.getU16()
      const expected = serializeKvpList(setupOptions)
      expect(Array.from(frozen.getBytes(frozen.remaining))).toEqual(Array.from(expected.toUint8Array()))
    })

    test('excess roundtrip', () => {
      // Setup Options are bounded by Length, not by a count, so a caller reading
      // straight off the wire (rather than through ControlMessage.deserialize) must
      // slice to Length itself before parsing — this pins that contract.
      const setupOptions = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const setup = new Setup(setupOptions)
      const serialized = setup.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Setup))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = Setup.parsePayload(payload)
      expect(deserialized.setupOptions).toEqual(setupOptions)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message throws', () => {
      const setupOptions = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('Set me up!')),
      ]
      const setup = new Setup(setupOptions)
      const serialized = setup.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const buf = new FrozenByteBuffer(partial)
      expect(() => {
        buf.getVI()
        buf.getU16()
        Setup.parsePayload(buf)
      }).toThrow()
    })
  })
}
