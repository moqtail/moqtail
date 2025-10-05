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
import { NotEnoughBytesError, LengthExceedsMaxError } from '../error/error'

/**
 * Represents a protocol PublishNamespace message, used to announce a track and its parameters.
 *
 * @public
 */
export class PublishNamespace {
  /**
   * @public
   * Constructs a PublishNamespace message.
   *
   * @param requestId - The request ID for this publish namespace message.
   * @param trackNamespace - The track namespace as a Tuple.
   * @param parameters - The list of key-value parameters for the track.
   */
  constructor(
    public readonly requestId: bigint,
    public readonly trackNamespace: Tuple,
    public readonly parameters: KeyValuePair[],
  ) {}

  /**
   * @public
   * Gets the message type for this PublishNamespace message.
   *
   * @returns The ControlMessageType.Announce value.
   */
  getType(): ControlMessageType {
    return ControlMessageType.PublishNamespace
  }

  /**
   * @public
   * Serializes the PublishNamespace message into a {@link FrozenByteBuffer}.
   *
   * @returns The serialized buffer.
   * @throws :{@link LengthExceedsMaxError} If the payload exceeds 65535 bytes.
   */
  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishNamespace)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putTuple(this.trackNamespace)
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param)
    }
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishNamespace::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  /**
   * @public
   * Parses a PublishNamespace message from the given buffer.
   *
   * @param buf - The buffer to parse from.
   * @returns The parsed PublishNamespace message.
   * @throws :{@link NotEnoughBytesError} If the buffer does not contain enough bytes.
   */
  static parsePayload(buf: BaseByteBuffer): PublishNamespace {
    const requestId = buf.getVI()
    const trackNamespace = buf.getTuple()
    const paramCount = buf.getNumberVI()
    const parameters: KeyValuePair[] = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      parameters[i] = buf.getKeyValuePair()
    }
    return new PublishNamespace(requestId, trackNamespace, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('PublishNamespace', () => {
    test('roundtrip', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('god/dayyum')
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const announce = new PublishNamespace(requestId, trackNamespace, parameters)
      const serialized = announce.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishNamespace))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishNamespace.parsePayload(frozen)
      expect(deserialized.requestId).toBe(announce.requestId)
      expect(deserialized.trackNamespace.equals(announce.trackNamespace)).toBe(true)
      expect(deserialized.parameters.length).toBe(announce.parameters.length)
      for (let i = 0; i < deserialized.parameters.length; i++) {
        expect(deserialized.parameters[i]).toEqual(announce.parameters[i])
      }
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('god/dayyum')
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const announce = new PublishNamespace(requestId, trackNamespace, parameters)
      const serialized = announce.serialize().toUint8Array()
      const excess = new Uint8Array(serialized.length + 3)
      excess.set(serialized)
      excess.set([9, 1, 1], serialized.length)
      const buf = new ByteBuffer()
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishNamespace))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishNamespace.parsePayload(frozen)
      expect(deserialized.requestId).toBe(announce.requestId)
      expect(deserialized.trackNamespace.equals(announce.trackNamespace)).toBe(true)
      expect(deserialized.parameters.length).toBe(announce.parameters.length)
      for (let i = 0; i < deserialized.parameters.length; i++) {
        expect(deserialized.parameters[i]).toEqual(announce.parameters[i])
      }
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('god/dayyum')
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const announce = new PublishNamespace(requestId, trackNamespace, parameters)
      const serialized = announce.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const buf = new ByteBuffer()
      buf.putBytes(partial)
      const frozen = buf.freeze()
      let threw = false
      try {
        frozen.getVI()
        frozen.getU16()
        PublishNamespace.parsePayload(frozen)
      } catch (e) {
        threw = true
        expect(e).toBeInstanceOf(NotEnoughBytesError)
      }
      expect(threw).toBe(true)
    })
  })
}
