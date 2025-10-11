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
import { Tuple } from '../common/tuple'
import { Location } from '../common/location'
import { KeyValuePair } from '../common/pair'
import { LengthExceedsMaxError } from '../error/error'
import { ControlMessageType } from './constant'

export class Publish {
  constructor(
    public readonly requestId: bigint,
    public readonly trackNamespace: Tuple,
    public readonly trackName: string,
    public readonly trackAlias: bigint,
    public readonly groupOrder: number,
    public readonly contentExists: number,
    public readonly largestLocation: Location | undefined,
    public readonly forward: number,
    public readonly parameters: KeyValuePair[],
  ) {}

  static new(
    requestId: bigint | number,
    trackNamespace: Tuple,
    trackName: string,
    trackAlias: bigint | number,
    groupOrder: number,
    contentExists: number,
    largestLocation: Location | undefined,
    forward: number,
    parameters: KeyValuePair[],
  ): Publish {
    return new Publish(
      BigInt(requestId),
      trackNamespace,
      trackName,
      BigInt(trackAlias),
      groupOrder,
      contentExists,
      largestLocation,
      forward,
      parameters,
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
    payload.putTuple(this.trackNamespace)
    payload.putVI(this.trackName.length)
    payload.putBytes(new TextEncoder().encode(this.trackName))
    payload.putVI(this.trackAlias)
    payload.putU8(this.groupOrder)
    payload.putU8(this.contentExists)
    if (this.largestLocation !== undefined) {
      payload.putBytes(this.largestLocation.serialize().toUint8Array())
    }
    payload.putU8(this.forward)
    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putKeyValuePair(param)
    }
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
    const trackNamespace = buf.getTuple()
    const trackNameLength = buf.getVI()
    const trackNameBytes = buf.getBytes(Number(trackNameLength))
    const trackName = new TextDecoder().decode(trackNameBytes)
    const trackAlias = buf.getVI()
    const groupOrder = buf.getU8()
    const contentExists = buf.getU8()
    let largestLocation: Location | undefined
    if (contentExists === 1) {
      largestLocation = Location.deserialize(buf)
    }
    const forward = buf.getU8()
    const paramCount = buf.getVI()
    const parameters: KeyValuePair[] = new Array(Number(paramCount))
    for (let i = 0; i < paramCount; i++) {
      parameters[i] = buf.getKeyValuePair()
    }
    return new Publish(
      requestId,
      trackNamespace,
      trackName,
      trackAlias,
      groupOrder,
      contentExists,
      largestLocation,
      forward,
      parameters,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Publish', () => {
    test('roundtrip without largest location', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('video/stream')
      const trackName = 'camera1'
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 0
      const largestLocation = undefined
      const forward = 1
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 100n),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('test')),
      ]
      const publish = Publish.new(
        requestId,
        trackNamespace,
        trackName,
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
      expect(deserialized.trackNamespace.equals(publish.trackNamespace)).toBe(true)
      expect(deserialized.trackName).toBe(publish.trackName)
      expect(deserialized.trackAlias).toBe(publish.trackAlias)
      expect(deserialized.groupOrder).toBe(publish.groupOrder)
      expect(deserialized.contentExists).toBe(publish.contentExists)
      expect(deserialized.largestLocation).toBe(undefined)
      expect(deserialized.forward).toBe(publish.forward)
      expect(deserialized.parameters.length).toBe(publish.parameters.length)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with largest location', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('video/stream')
      const trackName = 'camera1'
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 1
      const largestLocation = new Location(5n, 10n)
      const forward = 1
      const parameters = [
        KeyValuePair.tryNewVarInt(0, 100n),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('test')),
      ]
      const publish = Publish.new(
        requestId,
        trackNamespace,
        trackName,
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
      expect(deserialized.trackNamespace.equals(publish.trackNamespace)).toBe(true)
      expect(deserialized.trackName).toBe(publish.trackName)
      expect(deserialized.trackAlias).toBe(publish.trackAlias)
      expect(deserialized.groupOrder).toBe(publish.groupOrder)
      expect(deserialized.contentExists).toBe(publish.contentExists)
      expect(deserialized.largestLocation?.group).toBe(largestLocation.group)
      expect(deserialized.largestLocation?.object).toBe(largestLocation.object)
      expect(deserialized.forward).toBe(publish.forward)
      expect(deserialized.parameters.length).toBe(publish.parameters.length)
      expect(frozen.remaining).toBe(0)
    })

    test('excess roundtrip', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('video/stream')
      const trackName = 'camera1'
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 0
      const largestLocation = undefined
      const forward = 1
      const parameters = [KeyValuePair.tryNewVarInt(0, 100n)]
      const publish = Publish.new(
        requestId,
        trackNamespace,
        trackName,
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
      const deserialized = Publish.parsePayload(frozen)
      expect(deserialized.requestId).toBe(publish.requestId)
      expect(deserialized.trackNamespace.equals(publish.trackNamespace)).toBe(true)
      expect(deserialized.trackName).toBe(publish.trackName)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const requestId = 12345n
      const trackNamespace = Tuple.fromUtf8Path('video/stream')
      const trackName = 'camera1'
      const trackAlias = 123n
      const groupOrder = 1
      const contentExists = 0
      const largestLocation = undefined
      const forward = 1
      const parameters = [KeyValuePair.tryNewVarInt(0, 100n)]
      const publish = Publish.new(
        requestId,
        trackNamespace,
        trackName,
        trackAlias,
        groupOrder,
        contentExists,
        largestLocation,
        forward,
        parameters,
      )
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
