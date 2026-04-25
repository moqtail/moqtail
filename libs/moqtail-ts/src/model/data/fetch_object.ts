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
import { Location } from '../common/location'
import { ObjectForwardingPreference } from './constant'
import { ProtocolViolationError } from '../error/error'

// Draft-16 §10.4.4 Serialization Flags bit layout.
const FLAG_SUBGROUP_MODE_MASK = 0x03
const FLAG_OBJECT_ID_PRESENT = 0x04
const FLAG_GROUP_ID_PRESENT = 0x08
const FLAG_PRIORITY_PRESENT = 0x10
const FLAG_EXTENSIONS_PRESENT = 0x20
const FLAG_DATAGRAM = 0x40

const SUBGROUP_MODE_ZERO = 0b00
const SUBGROUP_MODE_PRIOR = 0b01
const SUBGROUP_MODE_PRIOR_PLUS_ONE = 0b10
const SUBGROUP_MODE_PRESENT = 0b11

// Special Serialization Flag varint values for End-of-Range markers.
const END_OF_NON_EXISTENT_RANGE = 0x8cn
const END_OF_UNKNOWN_RANGE = 0x10cn

/**
 * Prior-object state threaded across successive Fetch Objects on the same stream.
 */
export type FetchObjectContext = {
  groupId: bigint
  subgroupId: bigint
  objectId: bigint
  publisherPriority: number
}

/**
 * Kind discriminator for End-of-Range markers (§10.4.4.2).
 */
export enum EndOfRangeKind {
  NonExistent = 0x8c,
  Unknown = 0x10c,
}

/**
 * Payload-bearing fetch object (§10.4.4, non-End-of-Range form).
 *
 * Draft-16 FETCH objects carry no Object Status field; a zero-length payload
 * signals a zero-length Normal object.
 */
export class FetchObject {
  public readonly location: Location
  public readonly subgroupId: bigint

  private constructor(
    public readonly kind: 'object' | 'end_of_range',
    location: Location,
    subgroupId: bigint | number,
    public readonly publisherPriority: number,
    public readonly forwardingPreference: ObjectForwardingPreference,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly payload: Uint8Array | null,
    public readonly endOfRange: EndOfRangeKind | null,
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

  /**
   * Context to thread into the next call on this stream.
   * Returns null for EndOfRange markers — they MUST NOT update prior state.
   */
  toContext(): FetchObjectContext | null {
    if (this.kind === 'end_of_range') return null
    return {
      groupId: this.groupId,
      subgroupId: this.subgroupId,
      objectId: this.objectId,
      publisherPriority: this.publisherPriority,
    }
  }

  static newObject(
    groupId: bigint | number,
    subgroupId: bigint | number,
    objectId: bigint | number,
    publisherPriority: number,
    forwardingPreference: ObjectForwardingPreference,
    extensionHeaders: KeyValuePair[] | null,
    payload: Uint8Array,
  ): FetchObject {
    return new FetchObject(
      'object',
      new Location(groupId, objectId),
      subgroupId,
      publisherPriority,
      forwardingPreference,
      extensionHeaders,
      payload,
      null,
    )
  }

  static newEndOfRange(
    kind: EndOfRangeKind,
    groupId: bigint | number,
    objectId: bigint | number,
  ): FetchObject {
    return new FetchObject(
      'end_of_range',
      new Location(groupId, objectId),
      0,
      0,
      ObjectForwardingPreference.Subgroup,
      null,
      null,
      kind,
    )
  }

  serialize(prev?: FetchObjectContext): FrozenByteBuffer {
    const buf = new ByteBuffer()
    if (this.kind === 'end_of_range') {
      buf.putVI(this.endOfRange === EndOfRangeKind.NonExistent ? END_OF_NON_EXISTENT_RANGE : END_OF_UNKNOWN_RANGE)
      buf.putVI(this.groupId)
      buf.putVI(this.objectId)
      return buf.freeze()
    }

    const isDatagram = this.forwardingPreference === ObjectForwardingPreference.Datagram
    const hasExtensions = !!(this.extensionHeaders && this.extensionHeaders.length > 0)

    let hasGroupId: boolean
    let hasObjectId: boolean
    let hasPriority: boolean
    let subgroupMode: number

    if (!prev) {
      hasGroupId = true
      hasObjectId = true
      hasPriority = true
      subgroupMode = isDatagram || this.subgroupId === 0n ? SUBGROUP_MODE_ZERO : SUBGROUP_MODE_PRESENT
    } else {
      hasGroupId = prev.groupId !== this.groupId
      hasObjectId = prev.objectId + 1n !== this.objectId
      hasPriority = prev.publisherPriority !== this.publisherPriority
      if (isDatagram) {
        subgroupMode = SUBGROUP_MODE_ZERO
      } else if (this.subgroupId === 0n) {
        subgroupMode = SUBGROUP_MODE_ZERO
      } else if (prev.subgroupId === this.subgroupId) {
        subgroupMode = SUBGROUP_MODE_PRIOR
      } else if (prev.subgroupId + 1n === this.subgroupId) {
        subgroupMode = SUBGROUP_MODE_PRIOR_PLUS_ONE
      } else {
        subgroupMode = SUBGROUP_MODE_PRESENT
      }
    }

    let flags = subgroupMode & FLAG_SUBGROUP_MODE_MASK
    if (hasObjectId) flags |= FLAG_OBJECT_ID_PRESENT
    if (hasGroupId) flags |= FLAG_GROUP_ID_PRESENT
    if (hasPriority) flags |= FLAG_PRIORITY_PRESENT
    if (hasExtensions) flags |= FLAG_EXTENSIONS_PRESENT
    if (isDatagram) flags |= FLAG_DATAGRAM

    buf.putVI(flags)
    if (hasGroupId) buf.putVI(this.groupId)
    if (!isDatagram && subgroupMode === SUBGROUP_MODE_PRESENT) buf.putVI(this.subgroupId)
    if (hasObjectId) buf.putVI(this.objectId)
    if (hasPriority) buf.putU8(this.publisherPriority)
    if (hasExtensions) {
      const extBuf = new ByteBuffer()
      for (const h of this.extensionHeaders!) extBuf.putKeyValuePair(h)
      buf.putLengthPrefixedBytes(extBuf.toUint8Array())
    }
    const payloadBytes = this.payload ?? new Uint8Array(0)
    buf.putLengthPrefixedBytes(payloadBytes)
    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer, prev?: FetchObjectContext): FetchObject {
    const flagsRaw = buf.getVI()

    if (flagsRaw >= 128n) {
      let kind: EndOfRangeKind
      if (flagsRaw === END_OF_NON_EXISTENT_RANGE) {
        kind = EndOfRangeKind.NonExistent
      } else if (flagsRaw === END_OF_UNKNOWN_RANGE) {
        kind = EndOfRangeKind.Unknown
      } else {
        throw new ProtocolViolationError(
          'FetchObject.deserialize',
          `invalid Serialization Flags value 0x${flagsRaw.toString(16)}`,
        )
      }
      const groupId = buf.getVI()
      const objectId = buf.getVI()
      return FetchObject.newEndOfRange(kind, groupId, objectId)
    }

    const flags = Number(flagsRaw)
    const subgroupMode = flags & FLAG_SUBGROUP_MODE_MASK
    const hasObjectId = (flags & FLAG_OBJECT_ID_PRESENT) !== 0
    const hasGroupId = (flags & FLAG_GROUP_ID_PRESENT) !== 0
    const hasPriority = (flags & FLAG_PRIORITY_PRESENT) !== 0
    const hasExtensions = (flags & FLAG_EXTENSIONS_PRESENT) !== 0
    const isDatagram = (flags & FLAG_DATAGRAM) !== 0

    if (!prev) {
      if (!hasObjectId || !hasGroupId || !hasPriority) {
        throw new ProtocolViolationError(
          'FetchObject.deserialize',
          'first object must carry explicit group/object/priority',
        )
      }
      if (!isDatagram && (subgroupMode === SUBGROUP_MODE_PRIOR || subgroupMode === SUBGROUP_MODE_PRIOR_PLUS_ONE)) {
        throw new ProtocolViolationError(
          'FetchObject.deserialize',
          'first object cannot reference prior subgroup',
        )
      }
    }

    const groupId = hasGroupId
      ? buf.getVI()
      : (() => {
          if (!prev)
            throw new ProtocolViolationError(
              'FetchObject.deserialize',
              'group_id inherited but no prior object',
            )
          return prev.groupId
        })()

    let subgroupId: bigint
    if (isDatagram) {
      subgroupId = 0n
    } else {
      switch (subgroupMode) {
        case SUBGROUP_MODE_ZERO:
          subgroupId = 0n
          break
        case SUBGROUP_MODE_PRIOR:
          if (!prev)
            throw new ProtocolViolationError(
              'FetchObject.deserialize',
              'subgroup_id inherited but no prior object',
            )
          subgroupId = prev.subgroupId
          break
        case SUBGROUP_MODE_PRIOR_PLUS_ONE:
          if (!prev)
            throw new ProtocolViolationError(
              'FetchObject.deserialize',
              'subgroup_id inherited but no prior object',
            )
          subgroupId = prev.subgroupId + 1n
          break
        case SUBGROUP_MODE_PRESENT:
          subgroupId = buf.getVI()
          break
        default:
          throw new ProtocolViolationError(
            'FetchObject.deserialize',
            `invalid subgroup mode ${subgroupMode}`,
          )
      }
    }

    const objectId = hasObjectId
      ? buf.getVI()
      : (() => {
          if (!prev)
            throw new ProtocolViolationError(
              'FetchObject.deserialize',
              'object_id inherited but no prior object',
            )
          return prev.objectId + 1n
        })()

    const publisherPriority = hasPriority
      ? buf.getU8()
      : (() => {
          if (!prev)
            throw new ProtocolViolationError(
              'FetchObject.deserialize',
              'priority inherited but no prior object',
            )
          return prev.publisherPriority
        })()

    let extensionHeaders: KeyValuePair[] | null = null
    if (hasExtensions) {
      const extLen = buf.getNumberVI()
      const headerBytes = new FrozenByteBuffer(buf.getBytes(extLen))
      extensionHeaders = []
      while (headerBytes.remaining > 0) {
        extensionHeaders.push(headerBytes.getKeyValuePair())
      }
    }

    const payloadLen = buf.getNumberVI()
    const payload = buf.getBytes(payloadLen)

    const forwardingPreference = isDatagram
      ? ObjectForwardingPreference.Datagram
      : ObjectForwardingPreference.Subgroup

    // For Datagram forwarding, synthesize subgroup_id from object_id so the
    // unified Object view matches other ingress paths.
    const resolvedSubgroupId = isDatagram ? objectId : subgroupId

    return FetchObject.newObject(
      groupId,
      resolvedSubgroupId,
      objectId,
      publisherPriority,
      forwardingPreference,
      extensionHeaders,
      payload,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('FetchObject', () => {
    const samplePayload = () =>
      FetchObject.newObject(
        9n,
        144n,
        10n,
        255,
        ObjectForwardingPreference.Subgroup,
        [
          KeyValuePair.tryNewVarInt(0, 10),
          KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
        ],
        new TextEncoder().encode('01239gjawkk92837aldmi'),
      )

    test('roundtrip first object', () => {
      const obj = samplePayload()
      const frozen = obj.serialize()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.kind).toBe('object')
      expect(parsed.groupId).toBe(obj.groupId)
      expect(parsed.subgroupId).toBe(obj.subgroupId)
      expect(parsed.objectId).toBe(obj.objectId)
      expect(parsed.publisherPriority).toBe(obj.publisherPriority)
      expect(parsed.extensionHeaders).toEqual(obj.extensionHeaders)
      expect(parsed.payload).toEqual(obj.payload)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip inherited fields', () => {
      const first = samplePayload()
      const second = FetchObject.newObject(
        first.groupId,
        first.subgroupId,
        first.objectId + 1n,
        first.publisherPriority,
        ObjectForwardingPreference.Subgroup,
        null,
        new TextEncoder().encode('second'),
      )

      const wire = new ByteBuffer()
      wire.putBytes(first.serialize().toUint8Array())
      wire.putBytes(second.serialize(first.toContext()!).toUint8Array())
      const frozen = wire.freeze()

      const parsed1 = FetchObject.deserialize(frozen)
      expect(parsed1.groupId).toBe(first.groupId)
      expect(parsed1.objectId).toBe(first.objectId)
      const ctx1 = parsed1.toContext()!
      const parsed2 = FetchObject.deserialize(frozen, ctx1)
      expect(parsed2.groupId).toBe(second.groupId)
      expect(parsed2.subgroupId).toBe(second.subgroupId)
      expect(parsed2.objectId).toBe(second.objectId)
      expect(parsed2.publisherPriority).toBe(second.publisherPriority)
      expect(parsed2.payload).toEqual(second.payload)
      expect(frozen.remaining).toBe(0)
    })

    test('roundtrip datagram preference', () => {
      const obj = FetchObject.newObject(
        9n,
        10n, // subgroup_id = object_id for datagram round-trip
        10n,
        128,
        ObjectForwardingPreference.Datagram,
        null,
        new TextEncoder().encode('dgram'),
      )
      const frozen = obj.serialize()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.forwardingPreference).toBe(ObjectForwardingPreference.Datagram)
      expect(parsed.groupId).toBe(obj.groupId)
      expect(parsed.objectId).toBe(obj.objectId)
    })

    test('roundtrip end of non-existent range', () => {
      const obj = FetchObject.newEndOfRange(EndOfRangeKind.NonExistent, 7n, 42n)
      const frozen = obj.serialize()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.kind).toBe('end_of_range')
      expect(parsed.endOfRange).toBe(EndOfRangeKind.NonExistent)
      expect(parsed.groupId).toBe(7n)
      expect(parsed.objectId).toBe(42n)
    })

    test('roundtrip end of unknown range', () => {
      const obj = FetchObject.newEndOfRange(EndOfRangeKind.Unknown, 100n, 0n)
      const frozen = obj.serialize()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.kind).toBe('end_of_range')
      expect(parsed.endOfRange).toBe(EndOfRangeKind.Unknown)
    })

    test('reject reserved high flag value', () => {
      const buf = new ByteBuffer()
      buf.putVI(0x80)
      buf.putVI(1)
      buf.putVI(1)
      const frozen = buf.freeze()
      expect(() => FetchObject.deserialize(frozen)).toThrow(ProtocolViolationError)
    })

    test('reject first object inheriting priority', () => {
      // flags = subgroup_mode present | object_id present | group_id present, priority absent
      const flags = SUBGROUP_MODE_PRESENT | FLAG_OBJECT_ID_PRESENT | FLAG_GROUP_ID_PRESENT
      const buf = new ByteBuffer()
      buf.putVI(flags)
      buf.putVI(1)
      buf.putVI(1)
      buf.putVI(1)
      buf.putVI(0)
      const frozen = buf.freeze()
      expect(() => FetchObject.deserialize(frozen)).toThrow(ProtocolViolationError)
    })

    test('excess bytes preserved', () => {
      const obj = samplePayload()
      const serialized = obj.serialize().toUint8Array()
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(new Uint8Array([9, 1, 1]))
      const frozen = buf.freeze()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.groupId).toBe(obj.groupId)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message fails', () => {
      const obj = samplePayload()
      const serialized = obj.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => FetchObject.deserialize(frozen)).toThrow()
    })
  })
}
