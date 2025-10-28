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

export class SubgroupObject {
  public readonly objectId: bigint

  private constructor(
    objectId: bigint | number,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly objectStatus: ObjectStatus | null,
    public readonly payload: Uint8Array | null,
  ) {
    this.objectId = BigInt(objectId)
  }

  static newWithStatus(
    objectId: bigint | number,
    extensionHeaders: KeyValuePair[] | null,
    objectStatus: ObjectStatus,
  ): SubgroupObject {
    return new SubgroupObject(objectId, extensionHeaders, objectStatus, null)
  }

  static newWithPayload(
    objectId: bigint | number,
    extensionHeaders: KeyValuePair[] | null,
    payload: Uint8Array,
  ): SubgroupObject {
    return new SubgroupObject(objectId, extensionHeaders, null, payload)
  }

  // TODO: object delta encoding is missing here...
  serialize(previousObjectId: bigint | undefined): FrozenByteBuffer {
    // the first object's object id is encoded as is
    // for the subsequent objects, the object id is encoded
    // as the delta to the previous object id
    let objectIdDelta = previousObjectId ? this.objectId - previousObjectId - BigInt(1) : this.objectId

    const buf = new ByteBuffer()
    buf.putVI(objectIdDelta)
    const extensionHeaders = new ByteBuffer()
    if (this.extensionHeaders) {
      for (const header of this.extensionHeaders) {
        extensionHeaders.putKeyValuePair(header)
      }
      const headerBytes = extensionHeaders.toUint8Array()
      buf.putLengthPrefixedBytes(headerBytes)
    }
    if (this.payload) {
      buf.putLengthPrefixedBytes(this.payload)
    } else {
      buf.putVI(0)
      buf.putVI(this.objectStatus!)
    }
    return buf.freeze()
  }

  static deserialize(
    buf: BaseByteBuffer,
    hasExtensions: boolean,
    previousObjectId: bigint | undefined,
  ): SubgroupObject {
    const objectDelta = buf.getVI()
    let objectId = previousObjectId !== undefined ? previousObjectId + objectDelta + BigInt(1) : objectDelta
    let extensionHeaders: KeyValuePair[] | null = null
    if (hasExtensions) {
      extensionHeaders = []
      const extLen = buf.getNumberVI()
      const headerBytes = new FrozenByteBuffer(buf.getBytes(extLen))
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
    return new SubgroupObject(objectId, extensionHeaders, objectStatus, payload)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('SubgroupObject', () => {
    test('roundtrip', () => {
      const objectId = 10n
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const frozen = SubgroupObject.newWithPayload(objectId, extensionHeaders, payload).serialize(undefined)
      const parsed = SubgroupObject.deserialize(frozen, true, undefined)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.payload).toEqual(payload)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const objectId = 10n
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const serialized = SubgroupObject.newWithPayload(objectId, extensionHeaders, payload)
        .serialize(undefined)
        .toUint8Array()
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      const excess = new Uint8Array([9, 1, 1])
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const parsed = SubgroupObject.deserialize(frozen, true, undefined)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.payload).toEqual(payload)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message fails', () => {
      const objectId = 10n
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const serialized = SubgroupObject.newWithPayload(objectId, extensionHeaders, payload)
        .serialize(undefined)
        .toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        SubgroupObject.deserialize(frozen, true, undefined)
      }).toThrow()
    })
  })
}
