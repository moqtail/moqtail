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

/**
 * @public
 * PUBLISH_BLOCKED (0xF): a publisher tells the peer it cannot send the PUBLISH that
 * would start a subscription to a track under a SUBSCRIBE_TRACKS namespace, because it
 * is blocked by the peer's bidirectional stream limit. It MUST NOT send PUBLISH for
 * that track until the limit lifts (§10.20).
 *
 * Sent on the SUBSCRIBE_TRACKS response stream, so only the namespace suffix after that
 * subscription's prefix is carried.
 */
export class PublishBlocked {
  constructor(
    public readonly trackNamespaceSuffix: Tuple,
    public readonly trackName: Uint8Array,
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.PublishBlocked
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishBlocked)
    const payload = new ByteBuffer()
    payload.putTuple(this.trackNamespaceSuffix)
    payload.putLengthPrefixedBytes(this.trackName)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishBlocked::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): PublishBlocked {
    const trackNamespaceSuffix = buf.getTuple()
    const trackName = buf.getLengthPrefixedBytes()
    return new PublishBlocked(trackNamespaceSuffix, trackName)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('PublishBlocked', () => {
    const sample = () => new PublishBlocked(Tuple.fromUtf8Path('room1/audio'), new TextEncoder().encode('track-42'))

    test('roundtrip', () => {
      const msg = sample()
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishBlocked))
      expect(msgType).toBe(0x0fn)
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = PublishBlocked.parsePayload(frozen)
      expect(deserialized.trackNamespaceSuffix.equals(msg.trackNamespaceSuffix)).toBe(true)
      expect(deserialized.trackName).toEqual(msg.trackName)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const msg = sample()
      const buf = new ByteBuffer()
      buf.putBytes(msg.serialize().toUint8Array())
      buf.putBytes(new Uint8Array([9, 1, 1]))
      const frozen = buf.freeze()
      expect(frozen.getVI()).toBe(BigInt(ControlMessageType.PublishBlocked))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = PublishBlocked.parsePayload(frozen)
      expect(deserialized.trackNamespaceSuffix.equals(msg.trackNamespaceSuffix)).toBe(true)
      expect(deserialized.trackName).toEqual(msg.trackName)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const serialized = sample().serialize().toUint8Array()
      const frozen = new FrozenByteBuffer(serialized.slice(0, Math.floor(serialized.length / 2)))
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        PublishBlocked.parsePayload(frozen)
      }).toThrow()
    })

    test('a zero-element suffix round-trips', () => {
      const msg = new PublishBlocked(new Tuple(), new TextEncoder().encode('track-42'))
      const frozen = msg.serialize()
      frozen.getVI()
      frozen.getU16()
      const deserialized = PublishBlocked.parsePayload(frozen)
      expect(deserialized.trackNamespaceSuffix.fields.length).toBe(0)
      expect(deserialized.trackName).toEqual(msg.trackName)
    })
  })
}
