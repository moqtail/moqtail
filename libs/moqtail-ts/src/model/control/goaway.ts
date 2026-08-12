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
import { InvalidUTF8Error, LengthExceedsMaxError, NotEnoughBytesError, ProtocolViolationError } from '../error/error'

/** §10.4: a longer New Session URI is a protocol violation. */
export const MAX_NEW_SESSION_URI_LENGTH = 8192

/**
 * GOAWAY (0x10) winds down a session, or — on a request stream — migrates that one
 * request (§10.4).
 */
export class GoAway {
  newSessionUri?: string | undefined
  /**
   * Milliseconds the sender waits for graceful closure; 0 means no specific timeout.
   * On the control stream the sender closes the session with `GOAWAY_TIMEOUT` after it,
   * on a request stream it resets the stream with `GOING_AWAY`.
   */
  readonly timeout: bigint
  /**
   * The smallest peer Request ID that was not or might not have been processed. Present
   * only on the control stream, so a GOAWAY migrating a single request omits it.
   */
  readonly requestId?: bigint | undefined

  constructor(newSessionUri?: string, timeout: bigint = 0n, requestId?: bigint) {
    if (newSessionUri && newSessionUri.length === 0) {
      this.newSessionUri = undefined
    } else {
      this.newSessionUri = newSessionUri
    }
    this.timeout = timeout
    this.requestId = requestId
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
    payload.putVI(this.timeout)
    if (this.requestId !== undefined) payload.putVI(this.requestId)
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
    if (uriLength > MAX_NEW_SESSION_URI_LENGTH) {
      throw new ProtocolViolationError(
        'GoAway::parsePayload(uriLength)',
        `New Session URI length ${uriLength} exceeds ${MAX_NEW_SESSION_URI_LENGTH}`,
      )
    }
    let newSessionUri: string | undefined
    if (uriLength > 0) {
      if (buf.remaining < uriLength) {
        throw new NotEnoughBytesError('GoAway::parsePayload(uriLength)', uriLength, buf.remaining)
      }
      const uriBytes = buf.getBytes(uriLength)
      try {
        const decoder = new TextDecoder()
        newSessionUri = decoder.decode(uriBytes)
      } catch (error: unknown) {
        throw new InvalidUTF8Error(
          'GoAway::parsePayload(newSessionUri)',
          error instanceof Error ? error.message : String(error),
        )
      }
    }

    const timeout = buf.getVI()
    // Request ID is present only on the control stream, so it is optional and trailing.
    // The outer Length field is what bounds it.
    const requestId = buf.remaining > 0 ? buf.getVI() : undefined

    return new GoAway(newSessionUri, timeout, requestId)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('GoAway', () => {
    test('roundtrip with a request id, as sent on the control stream', () => {
      const goAway = new GoAway('Begone wreched monster', 5000n, 12n)
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
      expect(deserialized.timeout).toBe(5000n)
      expect(deserialized.requestId).toBe(12n)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip without a request id, as sent on a request stream', () => {
      const goAway = new GoAway(undefined, 250n)
      const frozen = goAway.serialize()
      frozen.getVI()
      frozen.getU16()
      const deserialized = GoAway.parsePayload(frozen)
      expect(deserialized.newSessionUri).toBeUndefined()
      expect(deserialized.timeout).toBe(250n)
      expect(deserialized.requestId).toBeUndefined()
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const goAway = new GoAway('Begone wreched monster', 0n, 4n)
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
      // The trailing Request ID is bounded by Length, so the payload must be sliced off
      // before parsing it — exactly what ControlMessage.deserialize does.
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = GoAway.parsePayload(payload)
      expect(deserialized.newSessionUri).toBe(goAway.newSessionUri)
      expect(deserialized.requestId).toBe(4n)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('a New Session URI over 8192 bytes is a protocol violation', () => {
      const payload = new ByteBuffer()
      payload.putVI(MAX_NEW_SESSION_URI_LENGTH + 1)
      expect(() => GoAway.parsePayload(payload.freeze())).toThrow(ProtocolViolationError)
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
