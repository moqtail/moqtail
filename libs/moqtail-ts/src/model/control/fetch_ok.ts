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

import { ByteBuffer, FrozenByteBuffer, BaseByteBuffer } from '../common/byte_buffer'
import { ControlMessageType } from './constant'
import { Location } from '../common/location'
import { LengthExceedsMaxError, NotEnoughBytesError, ProtocolViolationError } from '../error/error'
import {
  MessageParameter,
  MessageParameters,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { TrackProperty, ObjectDeliveryTimeoutProperty } from '../property/track_property'
import { ObjectDeliveryTimeout } from '../parameter/message/object_delivery_timeout'

/**
 * FETCH_OK (0x18) keeps a body of its own rather than folding into REQUEST_OK, but like
 * every other response it carries no Request ID: the request stream it arrives on
 * identifies the request (§10.1).
 */
export class FetchOk {
  constructor(
    public readonly endOfTrack: boolean,
    public readonly endLocation: Location,
    public readonly parameters: MessageParameter[],
    public readonly trackProperties: TrackProperty[] = [],
  ) {}

  getType(): ControlMessageType {
    return ControlMessageType.FetchOk
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(BigInt(ControlMessageType.FetchOk))
    const payload = new ByteBuffer()
    payload.putU8(this.endOfTrack ? 1 : 0)
    payload.putLocation(this.endLocation)
    payload.putVI(this.parameters.length)
    payload.putBytes(serializeMessageParameterKvps(this.parameters.map((p) => p.toKeyValuePair())).toUint8Array())
    TrackProperty.serializeInto(this.trackProperties, payload)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('FetchOk::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): FetchOk {
    if (buf.remaining < 1) {
      throw new NotEnoughBytesError('FetchOk::parsePayload(endOfTrack)', 1, 0)
    }
    const endOfTrackRaw = buf.getU8()
    let endOfTrack: boolean
    if (endOfTrackRaw === 0) {
      endOfTrack = false
    } else if (endOfTrackRaw === 1) {
      endOfTrack = true
    } else {
      throw new ProtocolViolationError(
        'FetchOk::parsePayload(endOfTrack)',
        'End of track must be true(0x01) or false(0x00)',
      )
    }
    const endLocation = buf.getLocation()
    const paramCount = buf.getNumberVI()
    const rawParams = deserializeMessageParameterKvps(buf, paramCount)
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)
    const trackProperties = TrackProperty.deserializeAll(buf)
    return new FetchOk(endOfTrack, endLocation, parameters, trackProperties)
  }
}

if (import.meta.vitest) {
  const { describe, expect, test } = import.meta.vitest

  describe('FetchOk', () => {
    test('roundtrip', () => {
      const endOfTrack = true
      const endLocation = new Location(17n, 57n)
      const parameters = [new ObjectDeliveryTimeout(200n)]
      const msg = new FetchOk(endOfTrack, endLocation, parameters)
      const frozen = msg.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.FetchOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const parsed = FetchOk.parsePayload(frozen)
      expect(parsed.endOfTrack).toBe(endOfTrack)
      expect(parsed.endLocation.equals(endLocation)).toBe(true)
      expect(parsed.parameters.length).toBe(1)
      expect(parsed.trackProperties.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with track properties', () => {
      const msg = new FetchOk(
        true,
        new Location(17n, 57n),
        [new ObjectDeliveryTimeout(200n)],
        [new ObjectDeliveryTimeoutProperty(8000n)],
      )
      const frozen = msg.serialize()
      frozen.getVI() // message type
      const msgLength = frozen.getU16()
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const parsed = FetchOk.parsePayload(payload)
      expect(parsed.trackProperties.length).toBe(1)
      expect(parsed.trackProperties[0]).toBeInstanceOf(ObjectDeliveryTimeoutProperty)
      expect(payload.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const endOfTrack = true
      const endLocation = new Location(17n, 57n)
      const parameters = [new ObjectDeliveryTimeout(200n)]
      const msg = new FetchOk(endOfTrack, endLocation, parameters)
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.FetchOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const parsed = FetchOk.parsePayload(payload)
      expect(parsed.endOfTrack).toBe(endOfTrack)
      expect(parsed.endLocation.equals(endLocation)).toBe(true)
      expect(parsed.parameters.length).toBe(1)
      expect(payload.remaining).toBe(0)
      expect(frozen.remaining).toBe(3)
    })

    test('partial message', () => {
      const endOfTrack = true
      const endLocation = new Location(17n, 57n)
      const parameters = [new ObjectDeliveryTimeout(200n)]
      const msg = new FetchOk(endOfTrack, endLocation, parameters)
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        FetchOk.parsePayload(frozen)
      }).toThrow()
    })
  })
}
