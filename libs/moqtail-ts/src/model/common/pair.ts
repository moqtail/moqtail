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
import { NotEnoughBytesError, LengthExceedsMaxError, KeyValueFormattingError } from '../error/error'

const MAX_VALUE_LENGTH = 2 ** 16 - 1 // 65535

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
   * @returns The serialized buffer.
   */
  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.typeValue)
    if (isVarInt(this)) {
      buf.putVI(this.value)
    } else if (isBytes(this)) {
      buf.putLengthPrefixedBytes(this.value)
    }
    return buf.freeze()
  }

  /**
   * Deserializes a KeyValuePair from a buffer.
   * @param buf - The buffer to read from.
   * @returns The deserialized KeyValuePair.
   * @throws LengthExceedsMaxError if blob length exceeds 65535 bytes.
   * @throws NotEnoughBytesError if buffer does not contain enough bytes.
   */
  static deserialize(buf: BaseByteBuffer): KeyValuePair {
    const typeValue = buf.getVI()
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
  })
}
