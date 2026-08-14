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
import { KeyValuePair, deserializeKvpListUntilEmpty, serializeKvpList } from '../common/pair'
import { Location } from '../common/location'
import { GroupOrder } from '../control/constant'
import { ObjectForwardingPreference } from './constant'
import { ProtocolViolationError } from '../error/error'

// §11.5 Serialization Flags bit layout.
const FLAG_SUBGROUP_MODE_MASK = 0x03
const FLAG_OBJECT_ID_PRESENT = 0x04
const FLAG_GROUP_ID_PRESENT = 0x08
const FLAG_PRIORITY_PRESENT = 0x10
const FLAG_PROPERTIES_PRESENT = 0x20
const FLAG_DATAGRAM = 0x40

const SUBGROUP_MODE_ZERO = 0b00
const SUBGROUP_MODE_PRIOR = 0b01
const SUBGROUP_MODE_PRIOR_PLUS_ONE = 0b10
const SUBGROUP_MODE_PRESENT = 0b11

// Special Serialization Flag varint values for End-of-Range markers.
const END_OF_NON_EXISTENT_RANGE = 0x8cn
const END_OF_UNKNOWN_RANGE = 0x10cn

const MAX_VARINT = 2n ** 64n - 1n

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
 * Kind discriminator for End-of-Range markers (§11.5).
 */
export enum EndOfRangeKind {
  NonExistent = 0x8c,
  Unknown = 0x10c,
}

/**
 * Payload-bearing fetch object (§11.5, non-End-of-Range form).
 *
 * FETCH objects carry no Object Status field; a zero-length payload
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
    public readonly properties: KeyValuePair[] | null,
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
    properties: KeyValuePair[] | null,
    payload: Uint8Array,
  ): FetchObject {
    return new FetchObject(
      'object',
      new Location(groupId, objectId),
      subgroupId,
      publisherPriority,
      forwardingPreference,
      properties,
      payload,
      null,
    )
  }

  static newEndOfRange(kind: EndOfRangeKind, groupId: bigint | number, objectId: bigint | number): FetchObject {
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

  serialize(prev?: FetchObjectContext, groupOrder: GroupOrder = GroupOrder.Original): FrozenByteBuffer {
    const buf = new ByteBuffer()
    if (this.kind === 'end_of_range') {
      buf.putVI(this.endOfRange === EndOfRangeKind.NonExistent ? END_OF_NON_EXISTENT_RANGE : END_OF_UNKNOWN_RANGE)
      buf.putVI(this.groupId)
      buf.putVI(this.objectId)
      return buf.freeze()
    }

    const isDatagram = this.forwardingPreference === ObjectForwardingPreference.Datagram
    const hasProperties = !!(this.properties && this.properties.length > 0)

    // Group ID Delta. Present on the first object (where it is the absolute Group ID)
    // and whenever the Group ID changes; absent when the Group ID is inherited.
    let groupDelta: bigint | null
    if (!prev) {
      groupDelta = this.groupId
    } else if (prev.groupId === this.groupId) {
      groupDelta = null
    } else {
      groupDelta =
        groupOrder === GroupOrder.Descending ? prev.groupId - this.groupId - 1n : this.groupId - prev.groupId - 1n
      if (groupDelta < 0n) {
        throw new ProtocolViolationError(
          'FetchObject.serialize',
          `group id not monotonic for ${GroupOrder[groupOrder]}: prior=${prev.groupId} current=${this.groupId}`,
        )
      }
    }

    // Object ID Delta: absolute on the first object and on a new group, omitted when it
    // is exactly prior + 1, otherwise the difference from the prior Object ID.
    let objectDelta: bigint | null
    if (!prev) {
      objectDelta = this.objectId
    } else if (groupDelta !== null) {
      objectDelta = this.objectId
    } else if (prev.objectId + 1n === this.objectId) {
      objectDelta = null
    } else {
      objectDelta = this.objectId - prev.objectId
      if (objectDelta < 0n) {
        throw new ProtocolViolationError(
          'FetchObject.serialize',
          `object id not monotonic: prior=${prev.objectId} current=${this.objectId}`,
        )
      }
    }

    const hasPriority = prev ? prev.publisherPriority !== this.publisherPriority : true

    let subgroupMode: number
    if (isDatagram || this.subgroupId === 0n) {
      subgroupMode = SUBGROUP_MODE_ZERO
    } else if (prev && prev.subgroupId === this.subgroupId) {
      subgroupMode = SUBGROUP_MODE_PRIOR
    } else if (prev && prev.subgroupId + 1n === this.subgroupId) {
      subgroupMode = SUBGROUP_MODE_PRIOR_PLUS_ONE
    } else {
      subgroupMode = SUBGROUP_MODE_PRESENT
    }

    let flags = subgroupMode & FLAG_SUBGROUP_MODE_MASK
    if (objectDelta !== null) flags |= FLAG_OBJECT_ID_PRESENT
    if (groupDelta !== null) flags |= FLAG_GROUP_ID_PRESENT
    if (hasPriority) flags |= FLAG_PRIORITY_PRESENT
    if (hasProperties) flags |= FLAG_PROPERTIES_PRESENT
    if (isDatagram) flags |= FLAG_DATAGRAM

    buf.putVI(flags)
    if (groupDelta !== null) buf.putVI(groupDelta)
    if (!isDatagram && subgroupMode === SUBGROUP_MODE_PRESENT) buf.putVI(this.subgroupId)
    if (objectDelta !== null) buf.putVI(objectDelta)
    if (hasPriority) buf.putU8(this.publisherPriority)
    if (hasProperties) {
      buf.putLengthPrefixedBytes(serializeKvpList(this.properties!).toUint8Array())
    }
    const payloadBytes = this.payload ?? new Uint8Array(0)
    buf.putLengthPrefixedBytes(payloadBytes)
    return buf.freeze()
  }

  static deserialize(
    buf: BaseByteBuffer,
    prev?: FetchObjectContext,
    groupOrder: GroupOrder = GroupOrder.Original,
  ): FetchObject {
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
    const hasProperties = (flags & FLAG_PROPERTIES_PRESENT) !== 0
    const isDatagram = (flags & FLAG_DATAGRAM) !== 0

    if (!prev) {
      if (!hasObjectId || !hasGroupId || !hasPriority) {
        throw new ProtocolViolationError(
          'FetchObject.deserialize',
          'first object must carry explicit group/object/priority',
        )
      }
      if (!isDatagram && (subgroupMode === SUBGROUP_MODE_PRIOR || subgroupMode === SUBGROUP_MODE_PRIOR_PLUS_ONE)) {
        throw new ProtocolViolationError('FetchObject.deserialize', 'first object cannot reference prior subgroup')
      }
    }

    // Group ID Delta present: absolute on the first object, otherwise applied to the
    // prior Group ID per the Group Order. A result outside [0, 2^64-1] closes the session.
    let groupId: bigint
    if (hasGroupId) {
      const delta = buf.getVI()
      if (!prev) {
        groupId = delta
      } else {
        groupId = groupOrder === GroupOrder.Descending ? prev.groupId - (delta + 1n) : prev.groupId + delta + 1n
        if (groupId < 0n || groupId > MAX_VARINT) {
          throw new ProtocolViolationError(
            'FetchObject.deserialize',
            `group id delta wraps: prior=${prev.groupId} delta=${delta} order=${GroupOrder[groupOrder]}`,
          )
        }
      }
    } else {
      if (!prev) throw new ProtocolViolationError('FetchObject.deserialize', 'group_id inherited but no prior object')
      groupId = prev.groupId
    }

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
            throw new ProtocolViolationError('FetchObject.deserialize', 'subgroup_id inherited but no prior object')
          subgroupId = prev.subgroupId
          break
        case SUBGROUP_MODE_PRIOR_PLUS_ONE:
          if (!prev)
            throw new ProtocolViolationError('FetchObject.deserialize', 'subgroup_id inherited but no prior object')
          subgroupId = prev.subgroupId + 1n
          break
        case SUBGROUP_MODE_PRESENT:
          subgroupId = buf.getVI()
          break
        default:
          throw new ProtocolViolationError('FetchObject.deserialize', `invalid subgroup mode ${subgroupMode}`)
      }
    }

    // Object ID resolution:
    // - delta present on the first object, or alongside a Group ID Delta: absolute.
    // - delta present without a Group ID Delta: prior Object ID + delta.
    // - delta absent: prior Object ID + 1.
    let objectId: bigint
    if (hasObjectId) {
      const delta = buf.getVI()
      if (!prev || hasGroupId) {
        objectId = delta
      } else {
        objectId = prev.objectId + delta
        if (objectId > MAX_VARINT) {
          throw new ProtocolViolationError(
            'FetchObject.deserialize',
            `object id delta wraps: prior=${prev.objectId} delta=${delta}`,
          )
        }
      }
    } else {
      if (!prev) throw new ProtocolViolationError('FetchObject.deserialize', 'object_id inherited but no prior object')
      objectId = prev.objectId + 1n
      if (objectId > MAX_VARINT) {
        throw new ProtocolViolationError('FetchObject.deserialize', `object id wraps: prior=${prev.objectId} +1`)
      }
    }

    const publisherPriority = hasPriority
      ? buf.getU8()
      : (() => {
          if (!prev)
            throw new ProtocolViolationError('FetchObject.deserialize', 'priority inherited but no prior object')
          return prev.publisherPriority
        })()

    let properties: KeyValuePair[] | null = null
    if (hasProperties) {
      const extLen = buf.getNumberVI()
      const headerBytes = new FrozenByteBuffer(buf.getBytes(extLen))
      properties = deserializeKvpListUntilEmpty(headerBytes)
    }

    const payloadLen = buf.getNumberVI()
    const payload = buf.getBytes(payloadLen)

    const forwardingPreference = isDatagram ? ObjectForwardingPreference.Datagram : ObjectForwardingPreference.Subgroup

    // For Datagram forwarding, synthesize subgroup_id from object_id so the
    // unified Object view matches other ingress paths.
    const resolvedSubgroupId = isDatagram ? objectId : subgroupId

    return FetchObject.newObject(
      groupId,
      resolvedSubgroupId,
      objectId,
      publisherPriority,
      forwardingPreference,
      properties,
      payload,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('FetchObject', () => {
    const u8 = (s: string) => new TextEncoder().encode(s)
    const samplePayload = () =>
      FetchObject.newObject(
        9n,
        144n,
        10n,
        255,
        ObjectForwardingPreference.Subgroup,
        [KeyValuePair.tryNewVarInt(0, 10), KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo'))],
        new TextEncoder().encode('01239gjawkk92837aldmi'),
      )

    // Round-trip tests share this encoder on both ends, so they cannot catch a field in
    // the wrong place. Pin the layout byte for byte against the Rust encoder.
    test('wire layout is byte exact', () => {
      const first = FetchObject.newObject(2n, 0n, 0n, 0, ObjectForwardingPreference.Subgroup, null, u8('abc'))
      expect(Array.from(first.serialize().toUint8Array())).toEqual([
        0x1c, // flags: subgroup zero, object id + group id + priority present
        0x02, // Group ID Delta — absolute on the first object
        // no Subgroup ID: the flags say it is zero
        0x00, // Object ID Delta — absolute on the first object
        0x00, // Publisher Priority
        // no Properties: the flags say absent
        0x03, // Object Payload Length
        0x61,
        0x62,
        0x63,
      ])

      const second = FetchObject.newObject(2n, 0n, 1n, 0, ObjectForwardingPreference.Subgroup, null, u8('de'))
      expect(Array.from(second.serialize(first.toContext()!).toUint8Array())).toEqual([0x00, 0x02, 0x64, 0x65])
    })

    test('roundtrip first object', () => {
      const obj = samplePayload()
      const frozen = obj.serialize()
      const parsed = FetchObject.deserialize(frozen)
      expect(parsed.kind).toBe('object')
      expect(parsed.groupId).toBe(obj.groupId)
      expect(parsed.subgroupId).toBe(obj.subgroupId)
      expect(parsed.objectId).toBe(obj.objectId)
      expect(parsed.publisherPriority).toBe(obj.publisherPriority)
      expect(parsed.properties).toEqual(obj.properties)
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

    // A run crossing a group boundary must round-trip, with the Object ID resetting
    // inside the new group and the Group ID recovered from its delta.
    test('roundtrip across group boundary', () => {
      const run = (groupA: bigint, groupC: bigint) => [
        FetchObject.newObject(groupA, 144n, 7n, 255, ObjectForwardingPreference.Subgroup, null, u8('a')),
        FetchObject.newObject(groupA, 144n, 8n, 255, ObjectForwardingPreference.Subgroup, null, u8('b')),
        FetchObject.newObject(groupC, 144n, 0n, 255, ObjectForwardingPreference.Subgroup, null, u8('c')),
      ]

      for (const order of [GroupOrder.Ascending, GroupOrder.Descending, GroupOrder.Original]) {
        // Descending needs decreasing group ids; flip the run for that order.
        const objs = order === GroupOrder.Descending ? run(6n, 4n) : run(4n, 6n)

        const wire = new ByteBuffer()
        let ctx: FetchObjectContext | undefined = undefined
        for (const o of objs) {
          wire.putBytes(o.serialize(ctx, order).toUint8Array())
          ctx = o.toContext() ?? ctx
        }
        const frozen = wire.freeze()

        ctx = undefined
        for (const o of objs) {
          const parsed: FetchObject = FetchObject.deserialize(frozen, ctx, order)
          expect(parsed.groupId, `order=${order}`).toBe(o.groupId)
          expect(parsed.objectId, `order=${order}`).toBe(o.objectId)
          expect(parsed.subgroupId, `order=${order}`).toBe(o.subgroupId)
          expect(parsed.payload, `order=${order}`).toEqual(o.payload)
          ctx = parsed.toContext() ?? ctx
        }
        expect(frozen.remaining, `order=${order}`).toBe(0)
      }
    })

    test('object id delta wrap closes session', () => {
      const prev: FetchObjectContext = {
        groupId: 3n,
        subgroupId: 0n,
        objectId: MAX_VARINT - 1n,
        publisherPriority: 5,
      }
      // Same group (group delta absent), object delta present = 5 → prior + 5 wraps.
      const buf = new ByteBuffer()
      buf.putVI(SUBGROUP_MODE_ZERO | FLAG_OBJECT_ID_PRESENT)
      buf.putVI(5)
      buf.putVI(0) // payload length
      expect(() => FetchObject.deserialize(buf.freeze(), prev)).toThrow(ProtocolViolationError)
    })

    test('ascending group id delta wrap closes session', () => {
      const prev: FetchObjectContext = {
        groupId: MAX_VARINT - 1n,
        subgroupId: 0n,
        objectId: 0n,
        publisherPriority: 5,
      }
      // Group delta present = 5 → prior + 5 + 1 wraps; object delta present (absolute).
      const buf = new ByteBuffer()
      buf.putVI(SUBGROUP_MODE_ZERO | FLAG_GROUP_ID_PRESENT | FLAG_OBJECT_ID_PRESENT)
      buf.putVI(5) // group delta
      buf.putVI(0) // object id (absolute)
      buf.putVI(0) // payload length
      expect(() => FetchObject.deserialize(buf.freeze(), prev, GroupOrder.Ascending)).toThrow(ProtocolViolationError)
    })

    test('descending group id delta underflow closes session', () => {
      const prev: FetchObjectContext = { groupId: 0n, subgroupId: 0n, objectId: 0n, publisherPriority: 5 }
      const buf = new ByteBuffer()
      buf.putVI(SUBGROUP_MODE_ZERO | FLAG_GROUP_ID_PRESENT | FLAG_OBJECT_ID_PRESENT)
      buf.putVI(0) // group delta → prior - (0 + 1) underflows
      buf.putVI(0)
      buf.putVI(0)
      expect(() => FetchObject.deserialize(buf.freeze(), prev, GroupOrder.Descending)).toThrow(ProtocolViolationError)
    })

    test('reject non-monotonic group id on serialize', () => {
      const prev: FetchObjectContext = { groupId: 9n, subgroupId: 0n, objectId: 0n, publisherPriority: 0 }
      const obj = FetchObject.newObject(4n, 0n, 0n, 0, ObjectForwardingPreference.Subgroup, null, u8(''))
      expect(() => obj.serialize(prev, GroupOrder.Ascending)).toThrow(ProtocolViolationError)
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
