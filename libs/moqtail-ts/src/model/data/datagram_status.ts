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
import { ObjectDatagramStatusType, ObjectStatus } from './constant'
import { Location } from '../common/location'

/**
 * Represents an OBJECT_DATAGRAM with status (Draft-14).
 *
 * Type values 0x20-0x21 indicate status datagrams:
 * - 0x20: Without extensions, Object ID present
 * - 0x21: With extensions, Object ID present
 *
 * Status datagrams always have Object ID present (unlike payload datagrams
 * which can omit Object ID when it's 0).
 */
export class DatagramStatus {
  public readonly trackAlias: bigint
  public readonly location: Location

  private constructor(
    public readonly type: ObjectDatagramStatusType,
    trackAlias: bigint | number,
    location: Location,
    public readonly publisherPriority: number,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly objectStatus: ObjectStatus,
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
   * Create a new DatagramStatus with all properties specified.
   * The type is automatically determined based on whether extensions are present.
   */
  static new(
    trackAlias: bigint | number,
    location: Location,
    publisherPriority: number,
    extensionHeaders: KeyValuePair[] | null,
    objectStatus: ObjectStatus,
  ): DatagramStatus {
    const hasExtensions = extensionHeaders !== null && extensionHeaders.length > 0
    const type = hasExtensions ? ObjectDatagramStatusType.WithExtensions : ObjectDatagramStatusType.WithoutExtensions

    return new DatagramStatus(
      type,
      trackAlias,
      location,
      publisherPriority,
      hasExtensions ? extensionHeaders : null,
      objectStatus,
    )
  }

  /**
   * Create a DatagramStatus with extensions.
   * @deprecated Use DatagramStatus.new() instead for Draft-14 compliance.
   */
  static withExtensions(
    trackAlias: bigint | number,
    location: Location,
    publisherPriority: number,
    extensionHeaders: KeyValuePair[],
    objectStatus: ObjectStatus,
  ): DatagramStatus {
    return DatagramStatus.new(trackAlias, location, publisherPriority, extensionHeaders, objectStatus)
  }

  /**
   * Create a DatagramStatus without extensions.
   * @deprecated Use DatagramStatus.new() instead for Draft-14 compliance.
   */
  static newWithoutExtensions(
    trackAlias: bigint | number,
    location: Location,
    publisherPriority: number,
    objectStatus: ObjectStatus,
  ): DatagramStatus {
    return DatagramStatus.new(trackAlias, location, publisherPriority, null, objectStatus)
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.type)
    buf.putVI(this.trackAlias)
    buf.putVI(this.location.group)
    buf.putVI(this.location.object) // Object ID is always present in status datagrams
    buf.putU8(this.publisherPriority)

    if (ObjectDatagramStatusType.hasExtensions(this.type)) {
      const extBuf = new ByteBuffer()
      if (this.extensionHeaders) {
        for (const header of this.extensionHeaders) {
          extBuf.putKeyValuePair(header)
        }
      }
      const extBytes = extBuf.toUint8Array()
      buf.putLengthPrefixedBytes(extBytes)
    }

    buf.putVI(this.objectStatus)
    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer): DatagramStatus {
    const msgTypeRaw = buf.getNumberVI()
    const msgType = ObjectDatagramStatusType.tryFrom(msgTypeRaw)
    const trackAlias = buf.getVI()
    const groupId = buf.getVI()
    const objectId = buf.getVI() // Object ID is always present in status datagrams
    const publisherPriority = buf.getU8()
    let extensionHeaders: KeyValuePair[] | null = null

    if (ObjectDatagramStatusType.hasExtensions(msgType)) {
      const extBytes = buf.getLengthPrefixedBytes()
      const headerBytes = new FrozenByteBuffer(extBytes)
      extensionHeaders = []
      while (headerBytes.remaining > 0) {
        extensionHeaders.push(headerBytes.getKeyValuePair())
      }
    }

    const objectStatus = ObjectStatus.tryFrom(buf.getVI())
    return new DatagramStatus(
      msgType,
      trackAlias,
      new Location(groupId, objectId),
      publisherPriority,
      extensionHeaders,
      objectStatus,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('DatagramStatus (Draft-14)', () => {
    test('roundtrip with extensions (type 0x21)', () => {
      const trackAlias = 144n
      const location = new Location(9n, 10n)
      const publisherPriority = 255
      const extensionHeaders = [
        KeyValuePair.tryNewVarInt(0, 10),
        KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
      ]
      const objectStatus = ObjectStatus.Normal
      const datagramStatus = DatagramStatus.new(trackAlias, location, publisherPriority, extensionHeaders, objectStatus)
      expect(datagramStatus.type).toBe(ObjectDatagramStatusType.WithExtensions)
      expect(datagramStatus.type).toBe(0x21)
      const frozen = datagramStatus.serialize()
      const parsed = DatagramStatus.deserialize(frozen)
      expect(parsed).toEqual(datagramStatus)
    })

    test('roundtrip without extensions (type 0x20)', () => {
      const trackAlias = 999n
      const location = new Location(5n, 42n)
      const publisherPriority = 128
      const objectStatus = ObjectStatus.DoesNotExist
      const datagramStatus = DatagramStatus.new(
        trackAlias,
        location,
        publisherPriority,
        null, // no extensions
        objectStatus,
      )
      expect(datagramStatus.type).toBe(ObjectDatagramStatusType.WithoutExtensions)
      expect(datagramStatus.type).toBe(0x20)
      const frozen = datagramStatus.serialize()
      const parsed = DatagramStatus.deserialize(frozen)
      expect(parsed).toEqual(datagramStatus)
    })

    test('deprecated factory methods still work', () => {
      const location = new Location(1n, 2n)
      const extensions = [KeyValuePair.tryNewVarInt(0, 1)]

      const withExt = DatagramStatus.withExtensions(1n, location, 100, extensions, ObjectStatus.Normal)
      expect(withExt.type).toBe(0x21)

      const noExt = DatagramStatus.newWithoutExtensions(2n, location, 200, ObjectStatus.Normal)
      expect(noExt.type).toBe(0x20)
    })
  })
}
