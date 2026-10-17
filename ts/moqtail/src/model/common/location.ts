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

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from './byte_buffer'
import { NotEnoughBytesError } from '../error/error'

/**
 * @public
 * Represents a position in a MOQT track, consisting of a group and object index.
 *
 * Used for specifying start/end positions in subscription and fetch requests.
 * - `group`: The group index (e.g. segment, GOP, or logical group).
 * - `object`: The object index within the group (e.g. frame, chunk).
 */
export class Location {
  /**
   * The group index for this location.
   */
  public readonly group: bigint

  /**
   * The object index within the group for this location.
   */
  public readonly object: bigint

  /**
   * Constructs a new Location.
   * @param group - The group index (number or bigint).
   * @param object - The object index (number or bigint).
   */
  constructor(group: bigint | number, object: bigint | number) {
    this.group = BigInt(group)
    this.object = BigInt(object)
  }

  /**
   * Serializes this Location to a FrozenByteBuffer.
   * @returns The serialized buffer.
   * @throws CastingError if group or object is negative.
   * @throws VarIntOverflowError if group or object exceeds varint encoding limits.
   */
  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.group)
    buf.putVI(this.object)
    return buf.freeze()
  }

  /**
   * Deserializes a Location from a buffer.
   * @param buf - The buffer to read from.
   * @returns The deserialized Location.
   * @throws NotEnoughBytesError if buffer does not contain enough bytes.
   */
  static deserialize(buf: BaseByteBuffer): Location {
    const group = buf.getVI()
    const object = buf.getVI()
    return new Location(group, object)
  }

  /**
   * Checks if this Location is equal to another.
   * @param other - The other Location to compare.
   * @returns True if both group and object are equal.
   */
  equals(other: Location): boolean {
    return this.group === other.group && this.object === other.object
  }

  /**
   * Compares this Location to another for ordering.
   * @param other - The other Location to compare.
   * @returns -1 if this \< other, 1 if this \> other, 0 if equal.
   */
  compare(other: Location): number {
    if (this.group < other.group) return -1
    if (this.group > other.group) return 1
    if (this.object < other.object) return -1
    if (this.object > other.object) return 1
    return 0
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  describe('Location', () => {
    it('roundtrip serialization and deserialization (FrozenByteBuffer)', () => {
      const original = new Location(BigInt(1), BigInt(100))
      const serialized = original.serialize()
      const parsed = Location.deserialize(serialized)
      expect(parsed.equals(original)).toBe(true)
    })

    it('roundtrip serialization and deserialization (ByteBuffer)', () => {
      const original = new Location(BigInt(42), BigInt(999))
      const serialized = original.serialize()
      const parsed = Location.deserialize(serialized)
      expect(parsed.equals(original)).toBe(true)
    })

    it('ordering comparison', () => {
      const loc1 = new Location(BigInt(1), BigInt(1))
      const loc2 = new Location(BigInt(1), BigInt(2))
      const loc3 = new Location(BigInt(2), BigInt(1))

      expect(loc1.compare(loc2)).toBeLessThanOrEqual(0)
      expect(loc2.compare(loc3)).toBeLessThanOrEqual(0)
      expect(loc1.compare(loc3)).toBeLessThanOrEqual(0)
      expect(loc2.compare(loc1)).toBe(1)
    })

    it('throws NotEnoughBytesError on insufficient data', () => {
      const buf = new ByteBuffer()
      buf.putVI(BigInt(1)) // missing second value
      const frozen = buf.freeze()
      expect(() => Location.deserialize(frozen)).toThrow(NotEnoughBytesError)
    })

    it('equality check works correctly', () => {
      const a = new Location(BigInt(1), BigInt(1))
      const b = new Location(BigInt(1), BigInt(1))
      const c = new Location(BigInt(2), BigInt(1))

      expect(a.equals(b)).toBe(true)
      expect(a.equals(c)).toBe(false)
    })

    it('handles large 64-bit values', () => {
      const maxVarint = BigInt('4611686018427387903')
      const loc = new Location(maxVarint, maxVarint)
      const serialized = loc.serialize()
      const parsed = Location.deserialize(serialized)
      expect(parsed.equals(loc)).toBe(true)
    })
  })
}
