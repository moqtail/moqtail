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
import { ControlMessageType } from './constant'
import { InvalidUTF8Error, LengthExceedsMaxError, NotEnoughBytesError } from '../error/error'

export class GoAway {
  newSessionUri?: string | undefined

  constructor(newSessionUri?: string) {
    if (newSessionUri && newSessionUri.length === 0) {
      this.newSessionUri = undefined
    } else {
      this.newSessionUri = newSessionUri
    }
  }

  getType(): ControlMessageType {
    return ControlMessageType.GoAway
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.GoAway)
    const payload = new ByteBuffer()
    if (this.newSessionUri) {
      let uriBytes: Uint8Array
      try {
        const encoder = new TextEncoder()
        uriBytes = encoder.encode(this.newSessionUri)
      } catch (error: unknown) {
        throw new InvalidUTF8Error(
          'GoAway::serialize(newSessionUri)',
          error instanceof Error ? error.message : String(error),
        )
      }
      payload.putLengthPrefixedBytes(uriBytes)
    } else {
      payload.putVI(0)
    }
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('GoAway::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): GoAway {
    const uriLength = buf.getNumberVI()
    if (uriLength === 0) {
      return new GoAway(undefined)
    }
    if (buf.remaining < uriLength) {
      throw new NotEnoughBytesError('GoAway::parsePayload(uriLength)', uriLength, buf.remaining)
    }
    const uriBytes = buf.getBytes(uriLength)

    let newSessionUri: string
    try {
      const decoder = new TextDecoder()
      newSessionUri = decoder.decode(uriBytes)
    } catch (error: unknown) {
      throw new InvalidUTF8Error(
        'GoAway::parsePayload(newSessionUri)',
        error instanceof Error ? error.message : String(error),
      )
    }

    return new GoAway(newSessionUri)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('GoAway', () => {
    test('roundtrip', () => {
      const newSessionUri = 'Begone wreched monster'
      const goAway = new GoAway(newSessionUri)
      const serialized = goAway.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.GoAway))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = GoAway.parsePayload(frozen)
      expect(deserialized.newSessionUri).toBe(goAway.newSessionUri)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const newSessionUri = 'Begone wreched monster'
      const goAway = new GoAway(newSessionUri)
      const serialized = goAway.serialize().toUint8Array()
      const excess = new Uint8Array(serialized.length + 3)
      excess.set(serialized, 0)
      excess.set([9, 1, 1], serialized.length)
      const buf = new ByteBuffer()
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.GoAway))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = GoAway.parsePayload(frozen)
      expect(deserialized.newSessionUri).toBe(goAway.newSessionUri)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const newSessionUri = 'Begone wreched monster'
      const goAway = new GoAway(newSessionUri)
      const serialized = goAway.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const buf = new ByteBuffer()
      buf.putBytes(partial)
      const frozen = buf.freeze()
      expect(() => {
        buf.getVI()
        buf.getU16()
        GoAway.parsePayload(frozen)
      }).toThrow()
    })
  })
}
