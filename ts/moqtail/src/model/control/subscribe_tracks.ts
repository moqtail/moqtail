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
import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { Tuple } from '../common/tuple'
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'
import {
  MessageParameter,
  MessageParameters,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { AuthorizationToken } from '../parameter/common/authorization_token'
import { Forward } from '../parameter/message/forward'

/**
 * SUBSCRIBE_TRACKS (0x51) asks for a PUBLISH for every track under a matching
 * namespace prefix, present and future (§10.19).
 *
 * Shares SUBSCRIBE_NAMESPACE's wire shape but has its own prefix-overlap space: the
 * two types may hold the same prefix, two of the same type may not.
 */
export class SubscribeTracks {
  constructor(
    public readonly requestId: bigint,
    public readonly trackNamespacePrefix: Tuple,
    public readonly parameters: MessageParameter[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.SubscribeTracks
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.SubscribeTracks)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putTuple(this.trackNamespacePrefix)
    payload.putVI(this.parameters.length)
    payload.putBytes(serializeMessageParameterKvps(this.parameters.map((p) => p.toKeyValuePair())).toUint8Array())
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('SubscribeTracks::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): SubscribeTracks {
    const requestId = buf.getVI()
    const trackNamespacePrefix = buf.getTuple()
    const paramCount = buf.getNumberVI()
    const rawParams = deserializeMessageParameterKvps(buf, paramCount)
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)
    return new SubscribeTracks(requestId, trackNamespacePrefix, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SubscribeTracks', () => {
    const getTestParameters = () => [
      AuthorizationToken.newUseValue(0n, new TextEncoder().encode('test-token')),
      new Forward(true),
    ]
    test('roundtrip', () => {
      const requestId = 241421n
      const trackNamespacePrefix = Tuple.fromUtf8Path('pre/fix/me')
      const msg = new SubscribeTracks(requestId, trackNamespacePrefix, getTestParameters())
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.SubscribeTracks))
      expect(msgType).toBe(0x51n)
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = SubscribeTracks.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.trackNamespacePrefix.equals(msg.trackNamespacePrefix)).toBe(true)
      expect(deserialized.parameters).toEqual(msg.parameters)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const msg = new SubscribeTracks(241421n, Tuple.fromUtf8Path('pre/fix/me'), getTestParameters())
      const buf = new ByteBuffer()
      buf.putBytes(msg.serialize().toUint8Array())
      buf.putBytes(new Uint8Array([9, 1, 1]))
      const frozen = buf.freeze()
      expect(frozen.getVI()).toBe(BigInt(ControlMessageType.SubscribeTracks))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = SubscribeTracks.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.trackNamespacePrefix.equals(msg.trackNamespacePrefix)).toBe(true)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const msg = new SubscribeTracks(241421n, Tuple.fromUtf8Path('pre/fix/me'), getTestParameters())
      const serialized = msg.serialize().toUint8Array()
      const frozen = new FrozenByteBuffer(serialized.slice(0, Math.floor(serialized.length / 2)))
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        SubscribeTracks.parsePayload(frozen)
      }).toThrow()
    })
  })
}
