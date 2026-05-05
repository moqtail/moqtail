/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { KeyValuePair } from '../common/pair'
import { ControlMessageType } from './constant'
import { CastingError, LengthExceedsMaxError } from '../error/error'
import { MessageParameter } from '../parameter/message_parameter'

export class RequestOk {
  public readonly requestId: bigint
  public readonly parameters: MessageParameter[]

  constructor(requestId: bigint | number, parameters: MessageParameter[] = []) {
    this.requestId = BigInt(requestId)
    this.parameters = parameters
  }

  getType(): ControlMessageType {
    return ControlMessageType.RequestOk
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.RequestOk)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(BigInt(this.parameters.length))

    for (const param of this.parameters) {
      payload.putBytes(param.toKeyValuePair().serialize().toUint8Array())
    }

    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('RequestOk::serialize(payloadBytes)', 0xffff, payloadBytes.length)
    }

    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): RequestOk {
    const requestId = buf.getVI()
    const numParamsBig = buf.getVI()
    const numParams = Number(numParamsBig)
    if (BigInt(numParams) !== numParamsBig) {
      throw new CastingError('RequestOk.parsePayload numParams', 'bigint', 'number', `${numParamsBig}`)
    }

    const parameters: MessageParameter[] = []
    for (let i = 0; i < numParams; i++) {
      const kvp = KeyValuePair.deserialize(buf)
      const param = MessageParameter.fromKeyValuePair(kvp)
      if (param !== undefined) parameters.push(param)
    }

    return new RequestOk(requestId, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('RequestOk', () => {
    test('roundtrip', () => {
      const requestId = 42n
      const msg = new RequestOk(requestId)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = RequestOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.parameters.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const requestId = 1337n
      const msg = new RequestOk(requestId)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = RequestOk.parsePayload(frozen)
      expect(deserialized.requestId).toBe(msg.requestId)
      expect(deserialized.parameters.length).toBe(0)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message', () => {
      const msg = new RequestOk(99n)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        RequestOk.parsePayload(frozen)
      }).toThrow()
    })
  })
}
