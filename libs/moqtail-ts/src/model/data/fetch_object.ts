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
import { ObjectStatus, FetchObjectSerializationFlags } from './constant'
import { Location } from '../common/location'
import { ProtocolViolationError } from '../error/error'

/**
 * Prior object state on a fetch stream, used for delta encoding.
 */
export interface FetchObjectPriorState {
  groupId: bigint
  subgroupId: bigint | null
  objectId: bigint
  publisherPriority: number
}

/**
 * End-of-range marker on a fetch stream.
 */
export class FetchEndOfRange {
  public readonly groupId: bigint
  public readonly objectId: bigint
  public readonly isNonExistent: boolean

  constructor(groupId: bigint | number, objectId: bigint | number, isNonExistent: boolean) {
    this.groupId = BigInt(groupId)
    this.objectId = BigInt(objectId)
    this.isNonExistent = isNonExistent
  }
}

/**
 * Item on a fetch stream: either a normal object or an end-of-range marker.
 */
export type FetchStreamItem = FetchObject | FetchEndOfRange

export class FetchObject {
  public readonly location: Location
  public readonly subgroupId: bigint | null

  private constructor(
    location: Location,
    subgroupId: bigint | number | null,
    public readonly publisherPriority: number,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly objectStatus: ObjectStatus | null,
    public readonly payload: Uint8Array | null,
  ) {
    this.location = location
    this.subgroupId = subgroupId === null ? null : BigInt(subgroupId)
  }

  get groupId(): bigint {
    return this.location.group
  }
  get objectId(): bigint {
    return this.location.object
  }

  static newWithStatus(
    groupId: bigint | number,
    subgroupId: bigint | number | null,
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
    subgroupId: bigint | number | null,
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
  /**
   * Serialize this object with optional delta encoding based on prior object state.
   * @param prior - Prior object state for delta encoding (undefined for first object)
   */
  serialize(prior?: FetchObjectPriorState): FrozenByteBuffer {
    const buf = new ByteBuffer()

    // Compute serialization flags
    const flags = FetchObjectSerializationFlags.fromProperties(prior, {
      groupId: this.location.group,
      subgroupId: this.subgroupId,
      objectId: this.location.object,
      publisherPriority: this.publisherPriority,
      extensionHeaders: this.extensionHeaders,
    })

    // Write serialization flags
    buf.putVI(flags)

    // Write Group ID if explicit
    if (FetchObjectSerializationFlags.hasExplicitGroupId(flags)) {
      buf.putVI(this.location.group)
    }

    // Write Subgroup ID if not datagram
    if (!FetchObjectSerializationFlags.isDatagram(flags)) {
      const mode = FetchObjectSerializationFlags.subgroupMode(flags)
      if (mode === 0x03) {
        // Explicit subgroup ID
        buf.putVI(this.subgroupId ?? 0n)
      }
      // modes 0x00, 0x01, 0x02 don't write the field
    }

    // Write Object ID if explicit
    if (FetchObjectSerializationFlags.hasExplicitObjectId(flags)) {
      buf.putVI(this.location.object)
    }

    // Write Publisher Priority if explicit
    if (FetchObjectSerializationFlags.hasExplicitPriority(flags)) {
      buf.putU8(this.publisherPriority)
    }

    // Write Extensions if present
    if (FetchObjectSerializationFlags.hasExtensions(flags)) {
      const extensionHeaders = new ByteBuffer()
      if (this.extensionHeaders) {
        for (const header of this.extensionHeaders) {
          extensionHeaders.putKeyValuePair(header)
        }
      }
      const extBytes = extensionHeaders.toUint8Array()
      buf.putLengthPrefixedBytes(extBytes)
    }

    // Write payload or status
    if (this.payload) {
      buf.putLengthPrefixedBytes(this.payload)
    } else {
      buf.putVI(0)
      buf.putVI(this.objectStatus!)
    }

    return buf.freeze()
  }

  /**
   * Deserialize a fetch stream item (object or end-of-range marker).
   * Updates the prior state with the deserialized values.
   * @param buf - Buffer to deserialize from
   * @param prior - Prior object state (updated in-place after deserialization)
   */
  static deserializeItem(buf: BaseByteBuffer, prior?: FetchObjectPriorState): FetchStreamItem {
    const flags = buf.getNumberVI()

    // Validate flags
    FetchObjectSerializationFlags.tryFrom(flags)

    // Handle end-of-range markers
    if (FetchObjectSerializationFlags.isEndOfRange(flags)) {
      const groupId = buf.getVI()
      const objectId = buf.getVI()
      return new FetchEndOfRange(groupId, objectId, flags === FetchObjectSerializationFlags.END_OF_NON_EXISTENT_RANGE)
    }

    // Validate first-object constraints
    if (!prior) {
      if (!FetchObjectSerializationFlags.hasExplicitGroupId(flags)) {
        throw new ProtocolViolationError(
          'FetchObject.deserializeItem',
          'First object must have explicit Group ID (bit 0x08 set)',
        )
      }
      if (!FetchObjectSerializationFlags.isDatagram(flags)) {
        const mode = FetchObjectSerializationFlags.subgroupMode(flags)
        if (mode === 0x01 || mode === 0x02) {
          throw new ProtocolViolationError(
            'FetchObject.deserializeItem',
            `First object cannot use subgroup mode ${mode} (references prior)`,
          )
        }
      }
      if (!FetchObjectSerializationFlags.hasExplicitObjectId(flags)) {
        throw new ProtocolViolationError(
          'FetchObject.deserializeItem',
          'First object must have explicit Object ID (bit 0x04 set)',
        )
      }
      if (!FetchObjectSerializationFlags.hasExplicitPriority(flags)) {
        throw new ProtocolViolationError(
          'FetchObject.deserializeItem',
          'First object must have explicit Priority (bit 0x10 set)',
        )
      }
    }

    // Read Group ID
    const groupId = FetchObjectSerializationFlags.hasExplicitGroupId(flags) ? buf.getVI() : prior!.groupId

    // Read Subgroup ID
    let subgroupId: bigint | null
    if (FetchObjectSerializationFlags.isDatagram(flags)) {
      subgroupId = null
    } else {
      const mode = FetchObjectSerializationFlags.subgroupMode(flags)
      if (mode === 0x00) {
        subgroupId = 0n
      } else if (mode === 0x01) {
        subgroupId = prior!.subgroupId
      } else if (mode === 0x02) {
        subgroupId = prior!.subgroupId! + 1n
      } else {
        // mode === 0x03: explicit
        subgroupId = buf.getVI()
      }
    }

    // Read Object ID
    const objectId = FetchObjectSerializationFlags.hasExplicitObjectId(flags) ? buf.getVI() : prior!.objectId + 1n

    // Read Publisher Priority
    const publisherPriority = FetchObjectSerializationFlags.hasExplicitPriority(flags) ? buf.getU8() : prior!.publisherPriority

    // Read Extensions
    let extensionHeaders: KeyValuePair[] | null = null
    if (FetchObjectSerializationFlags.hasExtensions(flags)) {
      const extLen = buf.getNumberVI()
      if (extLen > 0) {
        const headerBytes = new FrozenByteBuffer(buf.getBytes(extLen))
        extensionHeaders = []
        while (headerBytes.remaining > 0) {
          extensionHeaders.push(headerBytes.getKeyValuePair())
        }
      }
    }

    // Read payload or status
    const payloadLen = buf.getNumberVI()
    let objectStatus: ObjectStatus | null = null
    let payload: Uint8Array | null = null
    if (payloadLen === 0) {
      objectStatus = ObjectStatus.tryFrom(buf.getVI())
    } else {
      payload = buf.getBytes(payloadLen)
    }

    const obj = new FetchObject(new Location(groupId, objectId), subgroupId, publisherPriority, extensionHeaders, objectStatus, payload)
    return obj
  }

  /**
   * Deserialize a single fetch object (convenience method for tests).
   * Note: This cannot be used for multiple objects on the same stream, as it doesn't track prior state.
   */
  static deserialize(buf: BaseByteBuffer): FetchObject {
    const item = FetchObject.deserializeItem(buf, undefined)
    if (item instanceof FetchEndOfRange) {
      throw new ProtocolViolationError('FetchObject.deserialize', 'Cannot deserialize end-of-range marker as object')
    }
    return item
  }

  /**
   * Convert this object to prior state for use in delta encoding the next object.
   */
  toPriorState(): FetchObjectPriorState {
    return {
      groupId: this.location.group,
      subgroupId: this.subgroupId,
      objectId: this.location.object,
      publisherPriority: this.publisherPriority,
    }
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
