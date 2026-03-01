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
import { ObjectDatagramType } from './constant'
import { Location } from '../common/location'

/**
 * Represents an OBJECT_DATAGRAM message (Draft-14).
 *
 * Type values 0x00-0x07 indicate payload datagrams with varying properties:
 * - Bit 0: Extensions Present
 * - Bit 1: End of Group
 * - Bit 2: Object ID NOT present (when set, Object ID is 0)
 */
export class DatagramObject {
  public readonly trackAlias: bigint
  public readonly location: Location

  private constructor(
    public readonly type: ObjectDatagramType,
    trackAlias: number | bigint,
    location: Location,
    public readonly publisherPriority: number,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly payload: Uint8Array,
    public readonly endOfGroup: boolean,
  ) {
    this.trackAlias = BigInt(trackAlias)
    this.location = location
  }

  get groupId(): bigint {
    return this.location.group
  }
  get objectId(): bigint {
    return this.location.object
  }

  /**
   * Create a new DatagramObject with all properties specified.
   * The type is automatically determined based on extensions, endOfGroup, and objectId.
   */
  static new(
    trackAlias: bigint,
    groupId: bigint,
    objectId: bigint,
    publisherPriority: number,
    extensionHeaders: KeyValuePair[] | null,
    payload: Uint8Array,
    endOfGroup: boolean = false,
  ): DatagramObject {
    const hasExtensions = extensionHeaders !== null && extensionHeaders.length > 0
    const objectIdIsZero = objectId === 0n
    const type = ObjectDatagramType.fromProperties(hasExtensions, endOfGroup, objectIdIsZero)

    return new DatagramObject(
      type,
      trackAlias,
      new Location(groupId, objectId),
      publisherPriority,
      hasExtensions ? extensionHeaders : null,
      payload,
      endOfGroup,
    )
  }

  /**
   * Create a DatagramObject with extensions.
   * @deprecated Use DatagramObject.new() instead for Draft-14 compliance.
   */
  static newWithExtensions(
    trackAlias: bigint,
    groupId: bigint,
    objectId: bigint,
    publisherPriority: number,
    extensionHeaders: KeyValuePair[],
    payload: Uint8Array,
    endOfGroup: boolean = false,
  ): DatagramObject {
    return DatagramObject.new(trackAlias, groupId, objectId, publisherPriority, extensionHeaders, payload, endOfGroup)
  }

  /**
   * Create a DatagramObject without extensions.
   * @deprecated Use DatagramObject.new() instead for Draft-14 compliance.
   */
  static newWithoutExtensions(
    trackAlias: bigint,
    groupId: bigint,
    objectId: bigint,
    publisherPriority: number,
    payload: Uint8Array,
    endOfGroup: boolean = false,
  ): DatagramObject {
    return DatagramObject.new(trackAlias, groupId, objectId, publisherPriority, null, payload, endOfGroup)
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.type)
    buf.putVI(this.trackAlias)
    buf.putVI(this.location.group)

    // Object ID is only present if type indicates it (bit 2 NOT set)
    if (ObjectDatagramType.hasObjectId(this.type)) {
      buf.putVI(this.location.object)
    }

    buf.putU8(this.publisherPriority)

    // Extensions are present if bit 0 is set
    if (ObjectDatagramType.hasExtensions(this.type)) {
      const extBuf = new ByteBuffer()
      if (this.extensionHeaders) {
        for (const header of this.extensionHeaders) {
          extBuf.putKeyValuePair(header)
        }
      }
      const extBytes = extBuf.toUint8Array()
      buf.putLengthPrefixedBytes(extBytes)
    }

    buf.putBytes(this.payload)
    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer): DatagramObject {
    const msgTypeRaw = buf.getNumberVI()
    const msgType = ObjectDatagramType.tryFrom(msgTypeRaw)
    const trackAlias = buf.getVI()
    const groupId = buf.getVI()

    // Object ID is only present if type indicates it (bit 2 NOT set)
    let objectId: bigint
    if (ObjectDatagramType.hasObjectId(msgType)) {
      objectId = buf.getVI()
    } else {
      objectId = 0n
    }

    const publisherPriority = buf.getU8()
    let extensionHeaders: KeyValuePair[] | null = null

    // Extensions are present if bit 0 is set
    if (ObjectDatagramType.hasExtensions(msgType)) {
      const extBytes = buf.getLengthPrefixedBytes()
      const headerBytes = new FrozenByteBuffer(extBytes)
      extensionHeaders = []
      while (headerBytes.remaining > 0) {
        extensionHeaders.push(headerBytes.getKeyValuePair())
      }
    }

    const payload = buf.getBytes(buf.remaining)
    const endOfGroup = ObjectDatagramType.isEndOfGroup(msgType)

    return new DatagramObject(
      msgType,
      trackAlias,
      new Location(groupId, objectId),
      publisherPriority,
      extensionHeaders,
      payload,
      endOfGroup,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('DatagramObject', () => {
    test('roundtrip with extensions and explicit objectId', () => {
      const trackAlias = 500n
      const groupId = 9n
      const objectId = 10n
      const publisherPriority = 255
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(2, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const datagramObject = DatagramObject.new(
        trackAlias,
        groupId,
        objectId,
        publisherPriority,
        extensionHeaders,
        payload,
        false, // not end of group
      )
      expect(datagramObject.type).toBe(ObjectDatagramType.Type0x01) // Has extensions, not EOG, has objectId
      const frozen = datagramObject.serialize()
      const parsed = DatagramObject.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.payload).toEqual(payload)
      expect(parsed.endOfGroup).toBe(false)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip with objectId = 0', () => {
      const trackAlias = 100n
      const groupId = 5n
      const objectId = 0n
      const publisherPriority = 128
      const payload = new TextEncoder().encode('hello')
      const datagramObject = DatagramObject.new(trackAlias, groupId, objectId, publisherPriority, null, payload, false)
      expect(datagramObject.type).toBe(ObjectDatagramType.Type0x04) // No extensions, not EOG, objectId = 0
      const frozen = datagramObject.serialize()
      const parsed = DatagramObject.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(0n)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.extensionHeaders).toBeNull()
      expect(parsed.payload).toEqual(payload)
      expect(parsed.endOfGroup).toBe(false)
    })

    test('roundtrip with end of group', () => {
      const trackAlias = 200n
      const groupId = 3n
      const objectId = 99n
      const publisherPriority = 0
      const payload = new TextEncoder().encode('final object')
      const datagramObject = DatagramObject.new(
        trackAlias,
        groupId,
        objectId,
        publisherPriority,
        null,
        payload,
        true, // end of group
      )
      expect(datagramObject.type).toBe(ObjectDatagramType.Type0x02) // No extensions, EOG, has objectId
      const frozen = datagramObject.serialize()
      const parsed = DatagramObject.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.endOfGroup).toBe(true)
    })

    test('roundtrip with end of group and objectId = 0', () => {
      const trackAlias = 300n
      const groupId = 1n
      const objectId = 0n
      const publisherPriority = 64
      const extensionHeaders = [KeyValuePair.tryNewVarInt(4, 12345)]
      const payload = new TextEncoder().encode('only object in group')
      const datagramObject = DatagramObject.new(
        trackAlias,
        groupId,
        objectId,
        publisherPriority,
        extensionHeaders,
        payload,
        true, // end of group
      )
      expect(datagramObject.type).toBe(ObjectDatagramType.Type0x07) // Has extensions, EOG, objectId = 0
      const frozen = datagramObject.serialize()
      const parsed = DatagramObject.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(0n)
      expect(parsed.endOfGroup).toBe(true)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
    })
  })
}
