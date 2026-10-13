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
import { ControlMessageType, LocationFilterType } from './constant'
import { LengthExceedsMaxError } from '../error/error'
import { FullTrackName } from '../data'
import {
  MessageParameter,
  MessageParameters,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { SubscriberPriority } from '../parameter/message/subscriber_priority'
import { LocationFilter } from '../parameter/message/location_filter'

export class Fetch {
  constructor(
    public readonly requestId: bigint,
    public readonly fullTrackName: FullTrackName,
    public readonly parameters: MessageParameter[],
  ) {}

  /**
   * The requested range. A FETCH without a LOCATION_FILTER parameter asks for the
   * whole track up to the Largest Object, the same as a {@link (LocationFilterType:enum).None} filter.
   */
  get locationFilter(): LocationFilter {
    return this.parameters.find(MessageParameter.isLocationFilter) ?? new LocationFilter(LocationFilterType.None)
  }

  getType(): ControlMessageType {
    return ControlMessageType.Fetch
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Fetch)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putFullTrackName(this.fullTrackName)
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

    const paramCount = buf.getNumberVI()
    const rawParams = deserializeMessageParameterKvps(buf, paramCount)
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)

    return new Fetch(requestId, fullTrackName, parameters)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  // Parameters are serialized sorted by type and parse back in that order, so they
  // are listed sorted here for the roundtrip comparison to hold.
  const sampleFetch = () =>
    new Fetch(161803n, FullTrackName.tryNew('un/deux/trois', 'quatre'), [
      new SubscriberPriority(42),
      new LocationFilter(LocationFilterType.AbsoluteRange, 12n, 5n, 20n, 3n),
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

    test('the range is carried in the location filter, not in message fields', () => {
      const fetch = sampleFetch()
      expect(fetch.locationFilter).toEqual(new LocationFilter(LocationFilterType.AbsoluteRange, 12n, 5n, 20n, 3n))

      const frozen = fetch.serialize()
      frozen.getVI()
      frozen.getU16()
      frozen.getVI()
      frozen.getFullTrackName()
      expect(frozen.getNumberVI()).toBe(fetch.parameters.length)
    })

    test('no location filter is the whole track', () => {
      const fetch = new Fetch(1n, FullTrackName.tryNew('ns', 'track'), [])
      expect(fetch.locationFilter.filterType).toBe(LocationFilterType.None)
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
