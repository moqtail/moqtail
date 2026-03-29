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

import { ByteBuffer, FrozenByteBuffer, BaseByteBuffer } from '../common/byte_buffer'
import { ProtocolViolationError } from '../error'
import { SubgroupHeaderType } from './constant'

// TODO: couple type and subgroup id
export class SubgroupHeader {
  readonly subgroupId: bigint | undefined
  readonly trackAlias: bigint
  readonly groupId: bigint

  constructor(
    readonly type: SubgroupHeaderType,
    trackAlias: bigint | number,
    groupId: bigint | number,
    subgroupId: bigint | number | undefined,
    readonly publisherPriority: number | undefined,
  ) {
    this.trackAlias = BigInt(trackAlias)
    this.groupId = BigInt(groupId)
    if (subgroupId !== undefined) {
      this.subgroupId = BigInt(subgroupId)
    } else {
      this.subgroupId = subgroupId
    }
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.type)
    buf.putVI(this.trackAlias)
    buf.putVI(this.groupId)
    if (SubgroupHeaderType.hasExplicitSubgroupId(this.type)) {
      if (this.subgroupId === undefined) {
        throw new ProtocolViolationError(
          'SubgroupHeader.serialize',
          'Subgroup_id field is required for this header type',
        )
      }
      buf.putVI(this.subgroupId)
    }
    // Publisher priority is omitted when hasDefaultPriority bit is set
    if (!SubgroupHeaderType.hasDefaultPriority(this.type)) {
      if (this.publisherPriority === undefined) {
        throw new ProtocolViolationError(
          'SubgroupHeader.serialize',
          'Publisher_priority field is required when DEFAULT_PRIORITY bit is not set',
        )
      }
      buf.putU8(this.publisherPriority)
    }
    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer): SubgroupHeader {
    const headerType = SubgroupHeaderType.tryFrom(buf.getNumberVI())
    const trackAlias = buf.getVI()
    const groupId = buf.getVI()
    let subgroupId: bigint | undefined
    if (SubgroupHeaderType.hasExplicitSubgroupId(headerType)) {
      subgroupId = buf.getVI()
    } else if (SubgroupHeaderType.isSubgroupIdZero(headerType)) {
      subgroupId = 0n
    }
    // Publisher priority is omitted when hasDefaultPriority bit is set
    let publisherPriority: number | undefined
    if (!SubgroupHeaderType.hasDefaultPriority(headerType)) {
      publisherPriority = buf.getU8()
    }
    return new SubgroupHeader(headerType, trackAlias, groupId, subgroupId, publisherPriority)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('SubgroupHeader', () => {
    test('roundtrip serialization/deserialization', () => {
      const headerType = SubgroupHeaderType.Type0x14
      const trackAlias = 87n
      const groupId = 9n
      const subgroupId = 11n
      const publisherPriority = 255
      const header = new SubgroupHeader(headerType, trackAlias, groupId, subgroupId, publisherPriority)
      const frozen = header.serialize()
      const parsed = SubgroupHeader.deserialize(frozen)
      expect(parsed.type).toBe(header.type)
      expect(parsed.trackAlias).toBe(header.trackAlias)
      expect(parsed.groupId).toBe(header.groupId)
      expect(parsed.subgroupId).toBe(header.subgroupId)
      expect(parsed.publisherPriority).toBe(header.publisherPriority)
      expect(frozen.remaining).toBe(0)
    })
    test('roundtrip with default priority', () => {
      const headerType = SubgroupHeaderType.Type0x30 // DEFAULT_PRIORITY bit set, subgroupId = 0
      const trackAlias = 87n
      const groupId = 9n
      const subgroupId = 0n // Type0x30 has subgroupId = 0
      const publisherPriority = undefined // Not included in wire format
      const header = new SubgroupHeader(headerType, trackAlias, groupId, subgroupId, publisherPriority)
      const frozen = header.serialize()
      const parsed = SubgroupHeader.deserialize(frozen)
      expect(parsed.type).toBe(header.type)
      expect(parsed.trackAlias).toBe(header.trackAlias)
      expect(parsed.groupId).toBe(header.groupId)
      expect(parsed.subgroupId).toBe(header.subgroupId)
      expect(parsed.publisherPriority).toBe(undefined)
      expect(frozen.remaining).toBe(0)
    })
    test('excess roundtrip', () => {
      const headerType = SubgroupHeaderType.Type0x14
      const trackAlias = 87n
      const groupId = 9n
      const subgroupId = 11n
      const publisherPriority = 255
      const header = new SubgroupHeader(headerType, trackAlias, groupId, subgroupId, publisherPriority)
      const serialized = header.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const parsed = SubgroupHeader.deserialize(frozen)
      expect(parsed.type).toBe(header.type)
      expect(parsed.trackAlias).toBe(header.trackAlias)
      expect(parsed.groupId).toBe(header.groupId)
      expect(parsed.subgroupId).toBe(header.subgroupId)
      expect(parsed.publisherPriority).toBe(header.publisherPriority)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })
    test('partial message fails', () => {
      const headerType = SubgroupHeaderType.Type0x14
      const trackAlias = 87n
      const groupId = 9n
      const subgroupId = 11n
      const publisherPriority = 255
      const header = new SubgroupHeader(headerType, trackAlias, groupId, subgroupId, publisherPriority)
      const serialized = header.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        SubgroupHeader.deserialize(frozen)
      }).toThrow()
    })
  })
}
