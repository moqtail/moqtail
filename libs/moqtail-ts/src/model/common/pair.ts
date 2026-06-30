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
import {
  NotEnoughBytesError,
  LengthExceedsMaxError,
  KeyValueFormattingError,
  ProtocolViolationError,
} from '../error/error'

const MAX_VALUE_LENGTH = 2 ** 16 - 1 // 65535
const MAX_U64 = 2n ** 64n - 1n

/**
 * @public
 * Represents a key-value pair for MOQT protocol parameters.
 *
 * - If `typeValue` is **even**, the value is a varint (`bigint`).
 * - If `typeValue` is **odd**, the value is a binary blob (`Uint8Array`) with a maximum length of 65535 bytes.
 *
 * Use {@link KeyValuePair.tryNewVarInt} for varint pairs and {@link KeyValuePair.tryNewBytes} for blob pairs.
 */
export class KeyValuePair {
  /**
   * The key/type identifier for this pair.
   * - Even: value is a varint.
   * - Odd: value is a blob.
   */
  public readonly typeValue: bigint

  /**
   * The value for this pair.
   * - If `typeValue` is even: a varint (`bigint`).
   * - If `typeValue` is odd: a binary blob (`Uint8Array`).
   */
  public readonly value: bigint | Uint8Array

  /**
   * Constructs a new KeyValuePair.
   * @param typeValue - The key/type identifier.
   * @param value - The value (varint or blob).
   * @internal Use static factory methods instead.
   */
  private constructor(typeValue: bigint, value: bigint | Uint8Array) {
    this.typeValue = typeValue
    this.value = value
  }

  /**
   * Creates a new varint KeyValuePair.
   * @param typeValue - Must be even.
   * @param value - The varint value.
   * @returns A KeyValuePair with varint value.
   * @throws KeyValueFormattingError if typeValue is not even.
   */
  static tryNewVarInt(typeValue: bigint | number, value: bigint | number): KeyValuePair {
    const tv = typeof typeValue === 'number' ? BigInt(typeValue) : typeValue
    if (tv % 2n !== 0n) {
      throw new KeyValueFormattingError('KeyValuePair.tryNewVarInt')
    }
    const v = typeof value === 'number' ? BigInt(value) : value
    return new KeyValuePair(tv, v)
  }

  /**
   * Creates a new blob KeyValuePair.
   * @param typeValue - Must be odd.
   * @param value - The binary blob value.
   * @returns A KeyValuePair with blob value.
   * @throws KeyValueFormattingError if typeValue is not odd.
   * @throws LengthExceedsMaxError if value length exceeds 65535 bytes.
   */
  static tryNewBytes(typeValue: bigint | number, value: Uint8Array): KeyValuePair {
    const tv = typeof typeValue === 'number' ? BigInt(typeValue) : typeValue
    if (tv % 2n === 0n) {
      throw new KeyValueFormattingError('KeyValuePair.tryNewBytes')
    }
    const len = value.length
    if (len > MAX_VALUE_LENGTH) {
      throw new LengthExceedsMaxError('KeyValuePair.tryNewBytes', MAX_VALUE_LENGTH, len)
    }
    return new KeyValuePair(tv, value)
  }

  /**
   * Serializes this key-value pair to a frozen byte buffer.
   * Equivalent to {@link KeyValuePair.serializeDelta} with `prevType = 0n`.
   * @returns The serialized buffer.
   */
  serialize(): FrozenByteBuffer {
    return this.serializeDelta(0n)
  }

  /**
   * Serializes this pair's wire form, encoding the Type as a delta from
   * `prevType` (the type of the previous KVP in the same list, or 0n if this
   * is the first entry).
   * @param prevType - The type of the previous KVP in the same list.
   * @throws ProtocolViolationError if this pair's type is less than prevType
   *   (Delta Type is an unsigned varint, so the list must be sorted ascending).
   */
  serializeDelta(prevType: bigint): FrozenByteBuffer {
    if (this.typeValue < prevType) {
      throw new ProtocolViolationError(
        'KeyValuePair.serializeDelta',
        `type ${this.typeValue} is less than previous type ${prevType}; a KVP list must be sorted by ascending type to be delta-encoded`,
      )
    }
    const buf = new ByteBuffer()
    buf.putVI(this.typeValue - prevType)
    if (isVarInt(this)) {
      buf.putVI(this.value)
    } else if (isBytes(this)) {
      buf.putLengthPrefixedBytes(this.value)
    }
    return buf.freeze()
  }

  /**
   * Deserializes a KeyValuePair from a buffer.
   * Equivalent to {@link KeyValuePair.deserializeDelta} with `prevType = 0n`.
   * @param buf - The buffer to read from.
   * @returns The deserialized KeyValuePair.
   * @throws LengthExceedsMaxError if blob length exceeds 65535 bytes.
   * @throws NotEnoughBytesError if buffer does not contain enough bytes.
   */
  static deserialize(buf: BaseByteBuffer): KeyValuePair {
    return KeyValuePair.deserializeDelta(buf, 0n)
  }

  /**
   * Deserializes a wire-form KVP whose Type field is a delta from `prevType`
   * (the type of the previous KVP in the same list, or 0n if this is the
   * first entry).
   * @param buf - The buffer to read from.
   * @param prevType - The type of the previous KVP in the same list.
   * @throws ProtocolViolationError if prevType + delta exceeds 2^64 - 1.
   */
  static deserializeDelta(buf: BaseByteBuffer, prevType: bigint): KeyValuePair {
    const deltaType = buf.getVI()
    const typeValue = prevType + deltaType
    if (typeValue > MAX_U64) {
      throw new ProtocolViolationError(
        'KeyValuePair.deserializeDelta',
        `previous type ${prevType} plus delta type ${deltaType} exceeds 2^64 - 1`,
      )
    }
    if (typeValue % 2n === 0n) {
      const value = buf.getVI()
      return new KeyValuePair(typeValue, value)
    } else {
      const len = buf.getNumberVI()
      if (len > MAX_VALUE_LENGTH) {
        throw new LengthExceedsMaxError('KeyValuePair.deserialize', MAX_VALUE_LENGTH, len)
      }
      const value = buf.getBytes(len)
      return new KeyValuePair(typeValue, value)
    }
  }

  /**
   * Checks if this pair is equal to another.
   * @param other - The other KeyValuePair.
   * @returns True if both type and value are equal.
   */
  equals(other: KeyValuePair): boolean {
    if (this.typeValue !== other.typeValue) return false
    if (isVarInt(this) && isVarInt(other)) {
      return this.value === other.value
    }
    if (isBytes(this) && isBytes(other)) {
      const a = this.value
      const b = other.value
      if (a.length !== b.length) return false
      for (let i = 0; i < a.length; i++) {
        if (a[i] !== b[i]) return false
      }
      return true
    }
    return false
  }
}

/**
 * Delta-encodes and serializes a list of Key-Value-Pairs to wire bytes.
 * The list is sorted by ascending type first, since Delta Type is an
 * unsigned varint and cannot represent a type decrease.
 */
export function serializeKvpList(items: KeyValuePair[]): FrozenByteBuffer {
  const sorted = [...items].sort((a, b) => (a.typeValue < b.typeValue ? -1 : a.typeValue > b.typeValue ? 1 : 0))
  const buf = new ByteBuffer()
  let prevType = 0n
  for (const kvp of sorted) {
    buf.putBytes(kvp.serializeDelta(prevType).toUint8Array())
    prevType = kvp.typeValue
  }
  return buf.freeze()
}

/**
 * Reads exactly `count` delta-encoded Key-Value-Pairs from `buf`.
 */
export function deserializeKvpList(buf: BaseByteBuffer, count: number | bigint): KeyValuePair[] {
  const n = typeof count === 'bigint' ? Number(count) : count
  const items: KeyValuePair[] = new Array(n)
  let prevType = 0n
  for (let i = 0; i < n; i++) {
    const kvp = KeyValuePair.deserializeDelta(buf, prevType)
    prevType = kvp.typeValue
    items[i] = kvp
  }
  return items
}

/**
 * Reads delta-encoded Key-Value-Pairs from `buf` until it is exhausted.
 */
export function deserializeKvpListUntilEmpty(buf: BaseByteBuffer): KeyValuePair[] {
  const items: KeyValuePair[] = []
  let prevType = 0n
  while (buf.remaining > 0) {
    const kvp = KeyValuePair.deserializeDelta(buf, prevType)
    prevType = kvp.typeValue
    items.push(kvp)
  }
  return items
}

/**
 * Checks if the KeyValuePair is a varint pair (even typeValue).
 * @param pair - The KeyValuePair to check.
 * @returns True if value is a varint.
 */
export function isVarInt(pair: KeyValuePair): pair is KeyValuePair & { value: bigint } {
  return pair.typeValue % 2n === 0n
}

/**
 * Checks if the KeyValuePair is a blob pair (odd typeValue).
 * @param pair - The KeyValuePair to check.
 * @returns True if value is a Uint8Array.
 */
export function isBytes(pair: KeyValuePair): pair is KeyValuePair & { value: Uint8Array } {
  return pair.typeValue % 2n !== 0n
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  describe('KeyValuePair', () => {
    it('roundtrip varint', () => {
      const original = KeyValuePair.tryNewVarInt(2, 100)
      const serialized = original.serialize()
      const parsed = KeyValuePair.deserialize(serialized)
      expect(parsed).toEqual(original)
    })

    it('roundtrip bytes', () => {
      const data = new TextEncoder().encode('test')
      const original = KeyValuePair.tryNewBytes(1, data)
      const serialized = original.serialize()
      const parsed = KeyValuePair.deserialize(serialized)
      expect(parsed).toEqual(original)
    })

    it('invalid type for varint', () => {
      expect(() => KeyValuePair.tryNewVarInt(1, 100)).toThrow(KeyValueFormattingError)
    })

    it('invalid type for bytes', () => {
      const data = new Uint8Array([0x78])
      expect(() => KeyValuePair.tryNewBytes(2, data)).toThrow(KeyValueFormattingError)
    })

    it('length exceeds max', () => {
      const over = new Uint8Array(MAX_VALUE_LENGTH + 1)
      expect(() => KeyValuePair.tryNewBytes(1, over)).toThrow(LengthExceedsMaxError)
    })

    it('deserialize not enough bytes', () => {
      const buf = new ByteBuffer()
      buf.putVI(1) // odd -> bytes variant
      buf.putVI(5) // length = 5
      buf.putBytes(new Uint8Array([0x61, 0x62, 0x63])) // only 3 bytes
      const frozen = buf.freeze()
      expect(() => KeyValuePair.deserialize(frozen)).toThrow(NotEnoughBytesError)
    })

    it('deserialize length casting error', () => {
      const huge = BigInt(Number.MAX_SAFE_INTEGER) + 1n
      const buf = new ByteBuffer()
      buf.putVI(0)
      buf.putVI(huge)
      const frozen = buf.freeze()
      expect(() => KeyValuePair.deserialize(frozen))
    })

    it('deserializeDelta overflow is a protocol violation', () => {
      // previous_type + delta_type must not exceed 2^64 - 1.
      const buf = new ByteBuffer()
      buf.putVI(10n)
      const frozen = buf.freeze()
      expect(() => KeyValuePair.deserializeDelta(frozen, MAX_U64 - 5n)).toThrow(ProtocolViolationError)
    })

    it('serializeDelta with decreasing type is a protocol violation', () => {
      // Delta Type is an unsigned varint, so a type lower than prevType can't be encoded.
      const kvp = KeyValuePair.tryNewVarInt(2, 1)
      expect(() => kvp.serializeDelta(10n)).toThrow(ProtocolViolationError)
    })

    it('serializeKvpList sorts and delta-encodes', () => {
      const items = [
        KeyValuePair.tryNewVarInt(0x20, 1),
        KeyValuePair.tryNewVarInt(0x10, 2),
        KeyValuePair.tryNewBytes(0x21, new TextEncoder().encode('x')),
      ]
      const frozen = serializeKvpList(items)
      const decoded = deserializeKvpList(frozen, 3)
      expect(decoded.map((k) => k.typeValue)).toEqual([0x10n, 0x20n, 0x21n])
    })

    it('deserializeKvpListUntilEmpty reads a delta-encoded stream', () => {
      const items = [KeyValuePair.tryNewVarInt(0x04, 1), KeyValuePair.tryNewVarInt(0x02, 2)]
      const frozen = serializeKvpList(items)
      const decoded = deserializeKvpListUntilEmpty(frozen)
      expect(decoded.map((k) => k.typeValue)).toEqual([0x02n, 0x04n])
    })
  })
}
