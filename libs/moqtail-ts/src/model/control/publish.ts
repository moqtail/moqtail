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
import { LengthExceedsMaxError } from '../error/error'
import { ControlMessageType } from './constant'
import { FullTrackName } from '../data'
import { MessageParameter, MessageParameters } from '../parameter/message_parameter'
import { TrackExtension, DeliveryTimeoutExtension } from '../extension_header/track_extension'
import { DeliveryTimeout } from '../parameter/message/delivery_timeout'
import { Forward } from '../parameter/message/forward'

export class Publish {
  constructor(
    public readonly requestId: bigint,
    public readonly fullTrackName: FullTrackName,
    public readonly trackAlias: bigint,
    public readonly parameters: MessageParameter[],
    public readonly trackExtensions: TrackExtension[] = [],
  ) {}

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
    const paramCount = buf.getNumberVI()
    const rawParams = new Array(paramCount)
    for (let i = 0; i < paramCount; i++) {
      rawParams[i] = buf.getKeyValuePair()
    }
    const parameters = MessageParameters.fromKeyValuePairs(rawParams)
    const trackExtensions = TrackExtension.deserializeAll(buf)
    return new Publish(requestId, fullTrackName, trackAlias, parameters, trackExtensions)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Publish', () => {
    test('roundtrip', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const trackAlias = 123n
      const parameters = [new DeliveryTimeout(500n), new Forward(true)]
      const publish = new Publish(requestId, fullTrackName, trackAlias, parameters)

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
      expect(deserialized.parameters.length).toBe(2)
      expect(deserialized.trackExtensions.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip no parameters', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const trackAlias = 123n
      const publish = new Publish(requestId, fullTrackName, trackAlias, [])

      const frozen = publish.serialize()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.Publish))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining)
      const deserialized = Publish.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publish.requestId)
      expect(deserialized.parameters.length).toBe(0)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with track extensions', () => {
      const requestId = 12345n
      const fullTrackName = FullTrackName.tryNew('video/stream', 'camera1')
      const publish = new Publish(requestId, fullTrackName, 1n, [], [new DeliveryTimeoutExtension(3000n)])
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
      const parameters = [new DeliveryTimeout(500n)]
      const publish = new Publish(requestId, fullTrackName, trackAlias, parameters)
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
      const publish = new Publish(requestId, fullTrackName, 123n, [new DeliveryTimeout(500n)])
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
