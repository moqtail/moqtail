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
import { KeyValuePair } from '../common/pair'
import { ObjectStatus } from './constant'
import { Location } from '../common/location'

export class FetchObject {
  public readonly location: Location
  public readonly subgroupId: bigint

  private constructor(
    location: Location,
    subgroupId: bigint | number,
    public readonly publisherPriority: number,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly objectStatus: ObjectStatus | null,
    public readonly payload: Uint8Array | null,
  ) {
    this.location = location
    this.subgroupId = BigInt(subgroupId)
  }

  get groupId(): bigint {
    return this.location.group
  }
  get objectId(): bigint {
    return this.location.object
  }

  static newWithStatus(
    groupId: bigint | number,
    subgroupId: bigint | number,
    objectId: bigint | number,
    publisherPriority: number,
    extensionHeaders: KeyValuePair[] | null,
    objectStatus: ObjectStatus,
  ): FetchObject {
    return new FetchObject(
      new Location(groupId, objectId),
      subgroupId,
      publisherPriority,
      extensionHeaders,
      objectStatus,
      null,
    )
  }

  static newWithPayload(
    groupId: bigint | number,
    subgroupId: bigint | number,
    objectId: bigint | number,
    publisherPriority: number,
    extensionHeaders: KeyValuePair[] | null,
    payload: Uint8Array,
  ): FetchObject {
    return new FetchObject(
      new Location(groupId, objectId),
      subgroupId,
      publisherPriority,
      extensionHeaders,
      null,
      payload,
    )
  }
  // to be compatible with subgroup_object serialize method
  serialize(_previousObjectId?: BigInt): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.location.group)
    buf.putVI(this.subgroupId)
    buf.putVI(this.location.object)
    buf.putU8(this.publisherPriority)
    const extensionHeaders = new ByteBuffer()
    if (this.extensionHeaders) {
      for (const header of this.extensionHeaders) {
        extensionHeaders.putKeyValuePair(header)
      }
    }
    const extBytes = extensionHeaders.toUint8Array()
    buf.putLengthPrefixedBytes(extBytes)
    if (this.payload) {
      buf.putLengthPrefixedBytes(this.payload)
    } else {
      buf.putVI(0)
      buf.putVI(this.objectStatus!)
    }
    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer): FetchObject {
    const groupId = buf.getVI()
    const subgroupId = buf.getVI()
    const objectId = buf.getVI()
    const publisherPriority = buf.getU8()
    const extLen = buf.getNumberVI()
    let extensionHeaders: KeyValuePair[] | null = null
    if (extLen > 0) {
      const headerBytes = new FrozenByteBuffer(buf.getBytes(extLen))
      extensionHeaders = []
      while (headerBytes.remaining > 0) {
        extensionHeaders.push(headerBytes.getKeyValuePair())
      }
    }
    const payloadLen = buf.getNumberVI()
    let objectStatus: ObjectStatus | null = null
    let payload: Uint8Array | null = null
    if (payloadLen === 0) {
      objectStatus = ObjectStatus.tryFrom(buf.getVI())
    } else {
      payload = buf.getBytes(payloadLen)
    }
    return new FetchObject(
      new Location(groupId, objectId),
      subgroupId,
      publisherPriority,
      extensionHeaders,
      objectStatus,
      payload,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('FetchObject', () => {
    test('roundtrip', () => {
      const groupId = 9n
      const subgroupId = 144n
      const objectId = 10n
      const publisherPriority = 255
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 1000),
        KeyValuePair.tryNewBytes(9, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const fetchObject = FetchObject.newWithPayload(
        groupId,
        subgroupId,
        objectId,
        publisherPriority,
        extensionHeaders,
        payload,
      )
      const frozen = fetchObject.serialize()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.subgroupId).toBe(subgroupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.payload).toEqual(payload)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const groupId = 9n
      const subgroupId = 144n
      const objectId = 10n
      const publisherPriority = 255
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const fetchObject = FetchObject.newWithPayload(
        groupId,
        subgroupId,
        objectId,
        publisherPriority,
        extensionHeaders,
        payload,
      )
      const serialized = fetchObject.serialize().toUint8Array()
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      const excess = new Uint8Array([9, 1, 1])
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.subgroupId).toBe(subgroupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.payload).toEqual(payload)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message fails', () => {
      const groupId = 9n
      const subgroupId = 144n
      const objectId = 10n
      const publisherPriority = 255
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const fetchObject = FetchObject.newWithPayload(
        groupId,
        subgroupId,
        objectId,
        publisherPriority,
        extensionHeaders,
        payload,
      )
      const serialized = fetchObject.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        FetchObject.deserialize(frozen)
      }).toThrow()
    })
  })
}
