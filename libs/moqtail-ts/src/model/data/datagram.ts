/**
 * Copyright 2026 The MOQtail Authors
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
import { ObjectDatagramType, ObjectStatus } from './constant'
import { Location } from '../common/location'

/**
 * Represents a unified OBJECT_DATAGRAM message (Draft-16).
 *
 * Type bit layout (form 0b00X0XXXX):
 * - Bit 0 (0x01): EXTENSIONS
 * - Bit 1 (0x02): END_OF_GROUP
 * - Bit 2 (0x04): ZERO_OBJECT_ID
 * - Bit 3 (0x08): DEFAULT_PRIORITY
 * - Bit 5 (0x20): STATUS
 */
export class Datagram {
  public readonly trackAlias: bigint
  public readonly location: Location

  private constructor(
    public readonly type: ObjectDatagramType,
    trackAlias: number | bigint,
    location: Location,
    public readonly publisherPriority: number | null,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly payload: Uint8Array | null,
    public readonly objectStatus: ObjectStatus | null,
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
   * Create a new Datagram with payload content.
   * The type is automatically determined based on the properties.
   */
  static newPayload(
    trackAlias: bigint,
    groupId: bigint,
    objectId: bigint,
    publisherPriority: number | null,
    extensionHeaders: KeyValuePair[] | null,
    payload: Uint8Array,
    endOfGroup: boolean = false,
  ): Datagram {
    const hasExtensions = extensionHeaders !== null && extensionHeaders.length > 0
    const objectIdIsZero = objectId === 0n
    const defaultPriority = publisherPriority === null
    const type = ObjectDatagramType.fromProperties(hasExtensions, endOfGroup, objectIdIsZero, defaultPriority, false)

    return new Datagram(
      type,
      trackAlias,
      new Location(groupId, objectId),
      publisherPriority,
      hasExtensions ? extensionHeaders : null,
      payload,
      null,
      endOfGroup,
    )
  }

  /**
   * Create a new Datagram with Object Status (no payload).
   * endOfGroup is always false since STATUS + END_OF_GROUP is invalid.
   */
  static newStatus(
    trackAlias: bigint,
    groupId: bigint,
    objectId: bigint,
    publisherPriority: number | null,
    extensionHeaders: KeyValuePair[] | null,
    objectStatus: ObjectStatus,
  ): Datagram {
    const hasExtensions = extensionHeaders !== null && extensionHeaders.length > 0
    const objectIdIsZero = objectId === 0n
    const defaultPriority = publisherPriority === null
    const type = ObjectDatagramType.fromProperties(hasExtensions, false, objectIdIsZero, defaultPriority, true)

    return new Datagram(
      type,
      trackAlias,
      new Location(groupId, objectId),
      publisherPriority,
      hasExtensions ? extensionHeaders : null,
      null,
      objectStatus,
      false,
    )
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.type)
    buf.putVI(this.trackAlias)
    buf.putVI(this.location.group)

    // Object ID is present when ZERO_OBJECT_ID bit is NOT set
    if (!ObjectDatagramType.isZeroObjectId(this.type)) {
      buf.putVI(this.location.object)
    }

    // Publisher Priority is present when DEFAULT_PRIORITY bit is NOT set
    if (!ObjectDatagramType.hasDefaultPriority(this.type)) {
      buf.putU8(this.publisherPriority!)
    }

    // Extensions are present when EXTENSIONS bit is set
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

    // STATUS bit determines whether we write Object Status or Object Payload
    if (ObjectDatagramType.isStatus(this.type)) {
      buf.putVI(this.objectStatus!)
    } else {
      buf.putBytes(this.payload!)
    }

    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer): Datagram {
    const msgTypeRaw = buf.getNumberVI()
    const msgType = ObjectDatagramType.tryFrom(msgTypeRaw)
    const trackAlias = buf.getVI()
    const groupId = buf.getVI()

    // Object ID is present when ZERO_OBJECT_ID bit is NOT set
    let objectId: bigint
    if (ObjectDatagramType.isZeroObjectId(msgType)) {
      objectId = 0n
    } else {
      objectId = buf.getVI()
    }

    // Publisher Priority is present when DEFAULT_PRIORITY bit is NOT set
    let publisherPriority: number | null
    if (ObjectDatagramType.hasDefaultPriority(msgType)) {
      publisherPriority = null
    } else {
      publisherPriority = buf.getU8()
    }

    // Extensions are present when EXTENSIONS bit is set
    let extensionHeaders: KeyValuePair[] | null = null
    if (ObjectDatagramType.hasExtensions(msgType)) {
      const extBytes = buf.getLengthPrefixedBytes()
      const headerBytes = new FrozenByteBuffer(extBytes)
      extensionHeaders = []
      while (headerBytes.remaining > 0) {
        extensionHeaders.push(headerBytes.getKeyValuePair())
      }
    }

    // STATUS bit determines whether we read Object Status or Object Payload
    let payload: Uint8Array | null = null
    let objectStatus: ObjectStatus | null = null
    if (ObjectDatagramType.isStatus(msgType)) {
      objectStatus = ObjectStatus.tryFrom(buf.getVI())
    } else {
      payload = buf.getBytes(buf.remaining)
    }

    const endOfGroup = ObjectDatagramType.isEndOfGroup(msgType)

    return new Datagram(
      msgType,
      trackAlias,
      new Location(groupId, objectId),
      publisherPriority,
      extensionHeaders,
      payload,
      objectStatus,
      endOfGroup,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('Datagram (Draft-16)', () => {
    test('roundtrip payload with extensions and explicit objectId', () => {
      const trackAlias = 500n
      const groupId = 9n
      const objectId = 10n
      const publisherPriority = 255
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(2, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const payload = new TextEncoder().encode('01239gjawkk92837aldmi')
      const datagram = Datagram.newPayload(
        trackAlias,
        groupId,
        objectId,
        publisherPriority,
        extensionHeaders,
        payload,
        false,
      )
      expect(datagram.type).toBe(ObjectDatagramType.Type0x01) // Has extensions, not EOG, has objectId, no default priority
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.payload).toEqual(payload)
      expect(parsed.objectStatus).toBeNull()
      expect(parsed.endOfGroup).toBe(false)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip payload with objectId = 0 (ZERO_OBJECT_ID)', () => {
      const trackAlias = 100n
      const groupId = 5n
      const objectId = 0n
      const publisherPriority = 128
      const payload = new TextEncoder().encode('hello')
      const datagram = Datagram.newPayload(trackAlias, groupId, objectId, publisherPriority, null, payload, false)
      expect(datagram.type).toBe(ObjectDatagramType.Type0x04) // No extensions, not EOG, objectId = 0, no default priority
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(0n)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.extensionHeaders).toBeNull()
      expect(parsed.payload).toEqual(payload)
      expect(parsed.endOfGroup).toBe(false)
    })

    test('roundtrip payload with end of group', () => {
      const trackAlias = 200n
      const groupId = 3n
      const objectId = 99n
      const publisherPriority = 0
      const payload = new TextEncoder().encode('final object')
      const datagram = Datagram.newPayload(trackAlias, groupId, objectId, publisherPriority, null, payload, true)
      expect(datagram.type).toBe(ObjectDatagramType.Type0x02) // No extensions, EOG, has objectId, no default priority
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.endOfGroup).toBe(true)
    })

    test('roundtrip payload with DEFAULT_PRIORITY (null priority)', () => {
      const trackAlias = 42n
      const groupId = 1n
      const objectId = 5n
      const payload = new TextEncoder().encode('default priority')
      const datagram = Datagram.newPayload(trackAlias, groupId, objectId, null, null, payload, false)
      expect(datagram.type).toBe(ObjectDatagramType.Type0x08) // No extensions, not EOG, has objectId, default priority
      expect(ObjectDatagramType.hasDefaultPriority(datagram.type)).toBe(true)
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(groupId)
      expect(parsed.objectId).toBe(objectId)
      expect(parsed.publisherPriority).toBeNull()
      expect(parsed.payload).toEqual(payload)
    })

    test('roundtrip payload with all bits: extensions + EOG + ZERO_OBJECT_ID + DEFAULT_PRIORITY', () => {
      const trackAlias = 300n
      const groupId = 1n
      const objectId = 0n
      const extensionHeaders = [KeyValuePair.tryNewVarInt(4, 12345)]
      const payload = new TextEncoder().encode('all bits')
      const datagram = Datagram.newPayload(trackAlias, groupId, objectId, null, extensionHeaders, payload, true)
      expect(datagram.type).toBe(ObjectDatagramType.Type0x0F) // All payload bits set
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.objectId).toBe(0n)
      expect(parsed.publisherPriority).toBeNull()
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.endOfGroup).toBe(true)
      expect(parsed.payload).toEqual(payload)
      expect(parsed.objectStatus).toBeNull()
    })

    test('roundtrip status without extensions', () => {
      const trackAlias = 999n
      const location = new Location(5n, 42n)
      const publisherPriority = 128
      const datagram = Datagram.newStatus(
        trackAlias,
        location.group,
        location.object,
        publisherPriority,
        null,
        ObjectStatus.EndOfGroup,
      )
      expect(ObjectDatagramType.isStatus(datagram.type)).toBe(true)
      expect(datagram.type).toBe(ObjectDatagramType.Type0x20) // Status, no extensions, has objectId, no default priority
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(5n)
      expect(parsed.objectId).toBe(42n)
      expect(parsed.publisherPriority).toBe(publisherPriority)
      expect(parsed.objectStatus).toBe(ObjectStatus.EndOfGroup)
      expect(parsed.payload).toBeNull()
      expect(parsed.endOfGroup).toBe(false)
    })

    test('roundtrip status with extensions', () => {
      const trackAlias = 144n
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const datagram = Datagram.newStatus(
        trackAlias,
        9n,
        10n,
        255,
        extensionHeaders,
        ObjectStatus.EndOfTrack,
      )
      expect(datagram.type).toBe(ObjectDatagramType.Type0x21) // Status, with extensions, has objectId, no default priority
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.groupId).toBe(9n)
      expect(parsed.objectId).toBe(10n)
      expect(parsed.publisherPriority).toBe(255)
      expect(parsed.extensionHeaders).toEqual(extensionHeaders)
      expect(parsed.objectStatus).toBe(ObjectStatus.EndOfTrack)
    })

    test('roundtrip status with ZERO_OBJECT_ID and DEFAULT_PRIORITY', () => {
      const trackAlias = 77n
      const datagram = Datagram.newStatus(trackAlias, 3n, 0n, null, null, ObjectStatus.EndOfGroup)
      expect(datagram.type).toBe(ObjectDatagramType.Type0x2C) // Status + ZERO_OBJECT_ID + DEFAULT_PRIORITY
      const frozen = datagram.serialize()
      const parsed = Datagram.deserialize(frozen)
      expect(parsed.trackAlias).toBe(trackAlias)
      expect(parsed.objectId).toBe(0n)
      expect(parsed.publisherPriority).toBeNull()
      expect(parsed.objectStatus).toBe(ObjectStatus.EndOfGroup)
    })

    test('STATUS + END_OF_GROUP throws', () => {
      expect(() => {
        ObjectDatagramType.fromProperties(false, true, false, false, true)
      }).toThrow('PROTOCOL_VIOLATION')
    })

    test('invalid type value throws', () => {
      expect(() => {
        ObjectDatagramType.tryFrom(0x22) // STATUS + END_OF_GROUP
      }).toThrow('Invalid ObjectDatagramType')
      expect(() => {
        ObjectDatagramType.tryFrom(0x10) // Bit 4 set — subgroup header range
      }).toThrow('Invalid ObjectDatagramType')
    })
  })
}
