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
import { Location } from '../common/location'
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'
import { FullTrackName } from '../data'
import {
  MessageParameter,
  MessageParameters,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { SubscriberPriority } from '../parameter/message/subscriber_priority'

export class Fetch {
  constructor(
    public readonly requestId: bigint,
    public readonly fullTrackName: FullTrackName,
    public readonly startLocation: Location,
    /** The last Object plus 1. An Object value of 0 means the entire group. */
    public readonly endLocation: Location,
    public readonly parameters: MessageParameter[],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.Fetch
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Fetch)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putFullTrackName(this.fullTrackName)
    payload.putLocation(this.startLocation)
    payload.putLocation(this.endLocation)
    payload.putVI(this.parameters.length)
    payload.putBytes(serializeMessageParameterKvps(this.parameters.map((p) => p.toKeyValuePair())).toUint8Array())
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('Fetch::serialize(payload_length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): Fetch {
    const requestId = buf.getVI()
    const fullTrackName = buf.getFullTrackName()
    const startLocation = buf.getLocation()
    const endLocation = buf.getLocation()

    const paramCount = buf.getNumberVI()
    const rawParams = deserializeMessageParameterKvps(buf, paramCount)
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)

    return new Fetch(requestId, fullTrackName, startLocation, endLocation, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  const sampleFetch = () =>
    new Fetch(161803n, FullTrackName.tryNew('un/deux/trois', 'quatre'), new Location(12n, 5n), new Location(20n, 0n), [
      new SubscriberPriority(42),
    ])

  describe('Fetch', () => {
    test('roundtrip', () => {
      const fetch = sampleFetch()
      const serialized = fetch.serialize()
      const buf = new ByteBuffer()
      buf.putBytes(serialized.toUint8Array())
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Fetch))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = Fetch.parsePayload(frozen)
      expect(deserialized).toEqual(fetch)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const fetch = sampleFetch()
      const serialized = fetch.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Fetch))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const deserialized = Fetch.parsePayload(frozen)
      expect(deserialized).toEqual(fetch)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const fetch = sampleFetch()
      const serialized = fetch.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        Fetch.parsePayload(frozen)
      }).toThrow()
    })
  })
}
