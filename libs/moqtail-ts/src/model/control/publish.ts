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
import { LengthExceedsMaxError } from '../error/error'
import { ControlMessageType } from './constant'
import { FullTrackName } from '../data'
import { MessageParameter, MessageParameters } from '../parameter/message_parameter'
import { TrackExtension, DeliveryTimeoutExtension } from '../extension_header/track_extension'
import { DeliveryTimeout } from '../parameter/message/delivery_timeout'

export class Publish {
  constructor(
    public readonly requestId: bigint,
    public readonly fullTrackName: FullTrackName,
    public readonly trackAlias: bigint,
    public readonly groupOrder: number,
    public readonly contentExists: number,
    public readonly largestLocation: Location | undefined,
    public readonly forward: number,
    public readonly parameters: MessageParameter[],
    public readonly trackExtensions: TrackExtension[],
  ) {}

  static new(
    requestId: bigint | number,
    fullTrackName: FullTrackName,
    trackAlias: bigint | number,
    groupOrder: number,
    contentExists: number,
    largestLocation: Location | undefined,
    forward: number,
    parameters: MessageParameter[],
    trackExtensions: TrackExtension[] = [],
  ): Publish {
    return new Publish(
      BigInt(requestId),
      fullTrackName,
      BigInt(trackAlias),
      groupOrder,
      contentExists,
      largestLocation,
      forward,
      parameters,
      trackExtensions,
    )
  }

  getType(): ControlMessageType {
    return ControlMessageType.Publish
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Publish)
    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putBytes(this.fullTrackName.serialize().toUint8Array())
    payload.putVI(this.trackAlias)
    payload.putU8(this.groupOrder)
    payload.putU8(this.contentExists)
    if (this.largestLocation !== undefined) {
      payload.putBytes(this.largestLocation.serialize().toUint8Array())
    }
    payload.putU8(this.forward)
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param.toKeyValuePair())
    }
    TrackExtension.serializeInto(this.trackExtensions, payload)
    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('Publish::serialize(payloadBytes.length)', 0xffff, payloadBytes.length)
    }
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)
    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): Publish {
    const requestId = buf.getVI()
    const fullTrackName = buf.getFullTrackName()
    const trackAlias = buf.getVI()
    const groupOrder = buf.getU8()
    const contentExists = buf.getU8()
    let largestLocation: Location | undefined
    if (contentExists === 1) {
      largestLocation = Location.deserialize(buf)
    }
    const forward = buf.getU8()
    const paramCount = buf.getNumberVI()
    const rawParams = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      rawParams[i] = buf.getKeyValuePair()
    }
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)
    const trackExtensions = TrackExtension.deserializeAll(buf)
    return new Publish(
      requestId,
      fullTrackName,
      trackAlias,
      groupOrder,
      contentExists,
      largestLocation,
      forward,
      parameters,
      trackExtensions,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Publish', () => {
    test('roundtrip without largest location', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 0
      const largestLocation = undefined
      const forward = 1
      const parameters = [new DeliveryTimeout(500n)]
      const publish = Publish.new(
        requestId,
        fullTrackName,
        trackAlias,
        groupOrder,
        contentExists,
        largestLocation,
        forward,
        parameters,
      )

      const frozen = publish.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Publish))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = Publish.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publish.requestId)
      expect(deserialized.fullTrackName.namespace.equals(publish.fullTrackName.namespace)).toBe(true)
      expect(deserialized.fullTrackName.name).toEqual(publish.fullTrackName.name)
      expect(deserialized.trackAlias).toBe(publish.trackAlias)
      expect(deserialized.groupOrder).toBe(publish.groupOrder)
      expect(deserialized.contentExists).toBe(publish.contentExists)
      expect(deserialized.largestLocation).toBe(undefined)
      expect(deserialized.forward).toBe(publish.forward)
      expect(deserialized.parameters.length).toBe(1)
      expect(deserialized.trackExtensions.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with largest location', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 1
      const largestLocation = new Location(5n, 10n)
      const forward = 1
      const parameters = [new DeliveryTimeout(500n)]
      const publish = Publish.new(
        requestId,
        fullTrackName,
        trackAlias,
        groupOrder,
        contentExists,
        largestLocation,
        forward,
        parameters,
      )

      const frozen = publish.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Publish))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = Publish.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publish.requestId)
      expect(deserialized.largestLocation?.group).toBe(largestLocation.group)
      expect(deserialized.largestLocation?.object).toBe(largestLocation.object)
      expect(deserialized.parameters.length).toBe(1)
      expect(deserialized.trackExtensions.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with track extensions', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const publish = Publish.new(
        requestId,
        fullTrackName,
        1n,
        1,
        0,
        undefined,
        1,
        [],
        [new DeliveryTimeoutExtension(3000n)],
      )
      const frozen = publish.serialize()
      frozen.getVI() // message type
      const msgLength = frozen.getU16()
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = Publish.parsePayload(payload)
      expect(deserialized.trackExtensions.length).toBe(1)
      expect(deserialized.trackExtensions[0]).toBeInstanceOf(DeliveryTimeoutExtension)
      expect(payload.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 0
      const largestLocation = undefined
      const forward = 1
      const parameters = [new DeliveryTimeout(500n)]
      const publish = Publish.new(
        requestId,
        fullTrackName,
        trackAlias,
        groupOrder,
        contentExists,
        largestLocation,
        forward,
        parameters,
      )
      const serialized = publish.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Publish))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = Publish.parsePayload(payload)
      expect(deserialized.requestId).toBe(publish.requestId)
      expect(deserialized.fullTrackName.namespace.equals(publish.fullTrackName.namespace)).toBe(true)
      expect(deserialized.fullTrackName.name).toEqual(publish.fullTrackName.name)
      expect(payload.remaining).toBe(0)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const publish = Publish.new(requestId, fullTrackName, 123n, 1, 0, undefined, 1, [new DeliveryTimeout(500n)])
      const serialized = publish.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        Publish.parsePayload(frozen)
      }).toThrow()
    })
  })
}
