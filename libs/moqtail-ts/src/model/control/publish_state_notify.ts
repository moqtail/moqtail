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
import { Location } from '../common/location'

import { ControlMessageType } from './constant'
import { CastingError, LengthExceedsMaxError } from '../error/error'
import {
  MessageParameter,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { LargestObject } from '../parameter/message/largest_object'
import { Forward } from '../parameter/message/forward'

/**
 * @public
 * PUBLISH_STATE_NOTIFY (0x22): the publisher telling the subscriber that a
 * subscription's state changed for some reason other than the subscriber's own
 * REQUEST_UPDATE (§10.10).
 *
 * It carries no Request ID because it rides the subscription's own bidirectional
 * stream, and it carries only the parameters whose values changed; an absent one is
 * unchanged.
 *
 * Unlike REQUEST_UPDATE it is **unilateral**: the receiver sends no REQUEST_OK or
 * REQUEST_ERROR, and it is not counted against MAX_REQUEST_UPDATES. It is
 * informative, and no action is required of the recipient.
 *
 * It applies only to subscriptions and travels only publisher to subscriber.
 * Receiving one for any other request type, or from the subscriber, is a
 * PROTOCOL_VIOLATION that closes the session — a direction the codec cannot see, so
 * it is enforced where the stream's role is known.
 */
export class PublishStateNotify {
  constructor(public parameters: MessageParameter[]) {}

  getType(): ControlMessageType {
    return ControlMessageType.PublishStateNotify
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.PublishStateNotify)

    const payload = new ByteBuffer()
    payload.putVI(this.parameters.length)
    payload.putBytes(serializeMessageParameterKvps(this.parameters.map((p) => p.toKeyValuePair())).toUint8Array())

    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('PublishStateNotify::serialize(payloadBytes)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): PublishStateNotify {
    const paramCountBig = buf.getVI()
    const paramCount = Number(paramCountBig)
    if (BigInt(paramCount) !== paramCountBig) {
      throw new CastingError('PublishStateNotify.deserialize paramCount', 'bigint', 'number', `${paramCountBig}`)
    }

    const parameters: MessageParameter[] = []
    for (const kvp of deserializeMessageParameterKvps(buf, paramCount)) {
      const param = MessageParameter.fromKeyValuePair(kvp)
      if (param !== undefined) parameters.push(param)
    }

    return new PublishStateNotify(parameters)
  }

  equals(other: PublishStateNotify): boolean {
    if (this.parameters.length !== other.parameters.length) return false
    for (let i = 0; i < this.parameters.length; i++) {
      const a = this.parameters[i]
      const b = other.parameters[i]
      if (!a || !b) return false
      if (!a.toKeyValuePair().equals(b.toKeyValuePair())) return false
    }
    return true
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  describe('PublishStateNotify', () => {
    function sample(): PublishStateNotify {
      // Wire encoding canonicalizes parameter order ascending by type.
      const parameters = [new LargestObject(new Location(81n, 7n)), new Forward(false)].sort((a, b) => {
        const at = a.toKeyValuePair().typeValue
        const bt = b.toKeyValuePair().typeValue
        return at < bt ? -1 : at > bt ? 1 : 0
      })
      return new PublishStateNotify(parameters)
    }

    it('roundtrips', () => {
      const msg = sample()
      const buf = new ByteBuffer()
      buf.putBytes(msg.serialize().toUint8Array())

      const msgType = buf.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.PublishStateNotify))
      expect(msgType).toBe(0x22n)

      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining)

      const deserialized = PublishStateNotify.parsePayload(buf)
      expect(deserialized.equals(msg)).toBe(true)
      expect(buf.remaining).toBe(0)
    })

    it('roundtrips with excess trailing bytes', () => {
      const msg = sample()
      const buf = new ByteBuffer()
      buf.putBytes(new Uint8Array([...msg.serialize().toUint8Array(), 9, 1, 1]))

      expect(buf.getVI()).toBe(BigInt(ControlMessageType.PublishStateNotify))
      const msgLength = buf.getU16()
      expect(msgLength).toBe(buf.remaining - 3)

      expect(PublishStateNotify.parsePayload(buf).equals(msg)).toBe(true)
      expect(Array.from(buf.toUint8Array().slice(buf.offset))).toEqual([9, 1, 1])
    })

    it('throws on a partial message', () => {
      const bytes = sample().serialize().toUint8Array()
      const buf = new ByteBuffer()
      buf.putBytes(bytes.slice(0, Math.floor(bytes.length / 2)))
      expect(() => {
        buf.getVI()
        buf.getU16()
        PublishStateNotify.parsePayload(buf)
      }).toThrow()
    })

    it('carries no parameters when nothing changed', () => {
      // "If a parameter is not present, its value is unchanged", so an empty one says
      // nothing changed rather than being malformed.
      const msg = new PublishStateNotify([])
      const buf = new ByteBuffer()
      buf.putBytes(msg.serialize().toUint8Array())
      buf.getVI()
      buf.getU16()
      expect(PublishStateNotify.parsePayload(buf).equals(msg)).toBe(true)
    })

    it('carries no Request ID, because the stream it rides identifies the subscription', () => {
      // One varint of parameter count and nothing before it: a Request ID here would
      // be read as that count, so the absence is load-bearing rather than cosmetic.
      const buf = new ByteBuffer()
      buf.putBytes(new PublishStateNotify([new Forward(true)]).serialize().toUint8Array())
      buf.getVI()
      buf.getU16()
      expect(buf.getVI()).toBe(1n)
    })
  })
}
