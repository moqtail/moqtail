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
import { NotEnoughBytesError, CastingError } from '../error/error'

const PATH_SEPARATOR = '/'

/**
 * @public
 * Represents a single field in a Tuple, storing a value as a UTF-8 encoded byte array.
 *
 * Used for path segments or protocol fields in MOQT.
 */
export class TupleField {
  /**
   * The raw value of the field as a byte array.
   */
  constructor(public readonly value: Uint8Array) {}

  /**
   * Creates a TupleField from a UTF-8 string.
   * @param str - The string to encode.
   * @returns A new TupleField containing the encoded value.
   */
  static fromUtf8(str: string): TupleField {
    return new TupleField(new TextEncoder().encode(str))
  }

  /**
   * Decodes the field value to a UTF-8 string.
   * @returns The decoded string.
   */
  toUtf8(): string {
    return new TextDecoder().decode(this.value)
  }

  /**
   * Serializes this field to a length-prefixed byte array.
   * @returns The serialized bytes.
   */
  serialize(): Uint8Array {
    const buf = new ByteBuffer()
    buf.putVI(this.value.length)
    buf.putBytes(this.value)
    return buf.toUint8Array()
  }

  /**
   * Deserializes a TupleField from a buffer.
   * @param buf - The buffer to read from.
   * @returns The deserialized TupleField.
   * @throws CastingError if the length cannot be safely cast to number.
   * @throws NotEnoughBytesError if buffer does not contain enough bytes.
   */
  static deserialize(buf: BaseByteBuffer): TupleField {
    const lenBig = buf.getVI()
    const len = Number(lenBig)

    if (BigInt(len) !== lenBig) {
      throw new CastingError('TupleField.deserialize', 'bigint', 'number', `${lenBig}`)
    }

    const bytes = buf.getBytes(len)
    return new TupleField(bytes)
  }
}

/**
 * @public
 * Represents a sequence of TupleFields, typically used for hierarchical paths or protocol tuples in MOQT.
 */
export class Tuple {
  /**
   * The ordered list of fields in this tuple.
   */
  constructor(public readonly fields: TupleField[] = []) {}

  /**
   * Creates a Tuple from a path string, splitting on '/'.
   * @param path - The path string (e.g. '/foo/bar').
   * @returns A Tuple with each segment as a field.
   */
  static fromUtf8Path(path: string): Tuple {
    const parts = path.split(PATH_SEPARATOR).filter(Boolean)
    const fields = parts.map(TupleField.fromUtf8)
    return new Tuple(fields)
  }

  /**
   * Converts the tuple to a path string, joining fields with '/'.
   * @returns The path string (e.g. '/foo/bar').
   */
  toUtf8Path(): string {
    return this.fields.map((f) => PATH_SEPARATOR + f.toUtf8()).join('')
  }

  /**
   * Adds a field to the tuple.
   * @param field - The TupleField to add.
   */
  add(field: TupleField): void {
    this.fields.push(field)
  }

  /**
   * Gets the field at the specified index.
   * @param index - The field index.
   * @returns The TupleField at the index.
   * @throws Error if no field exists at the index.
   */
  get(index: number): TupleField {
    const field = this.fields[index]
    if (!field) throw new Error('Field not found at index')
    return field
  }

  /**
   * Sets the field at the specified index.
   * @param index - The field index.
   * @param field - The TupleField to set.
   */
  set(index: number, field: TupleField): void {
    this.fields[index] = field
  }

  /**
   * Clears all fields from the tuple.
   */
  clear(): void {
    this.fields.length = 0
  }

  /**
   * Serializes the tuple to a FrozenByteBuffer.
   * @returns The serialized buffer.
   */
  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(this.fields.length)
    for (const field of this.fields) {
      buf.putBytes(field.serialize())
    }
    return buf.freeze()
  }

  /**
   * Deserializes a Tuple from a buffer.
   * @param buf - The buffer to read from.
   * @returns The deserialized Tuple.
   * @throws CastingError if the count cannot be safely cast to number.
   * @throws NotEnoughBytesError if buffer does not contain enough bytes.
   */
  static deserialize(buf: BaseByteBuffer): Tuple {
    const countBig = buf.getVI()
    const count = Number(countBig)

    if (BigInt(count) !== countBig) {
      throw new CastingError('Tuple.deserialize', 'bigint', 'number', `${countBig}`)
    }

    const fields: TupleField[] = []
    for (let i = 0; i < count; i++) {
      const field = TupleField.deserialize(buf)
      fields.push(field)
    }

    return new Tuple(fields)
  }

  /**
   * Checks if this tuple is equal to another.
   * @param other - The other Tuple to compare.
   * @returns True if all fields are equal.
   */
  equals(other: Tuple): boolean {
    if (this.fields.length !== other.fields.length) return false
    return this.fields.every((f, i) => f.value.toString() === other.fields[i]!.value.toString())
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  describe('TupleField', () => {
    it('should serialize and deserialize correctly', () => {
      const field = TupleField.fromUtf8('hello')
      const serialized = field.serialize()

      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      const decoded = TupleField.deserialize(buf)

      expect(decoded.value).toEqual(field.value)
      expect(buf.remaining).toBe(0)
    })

    it('should throw on not enough bytes', () => {
      const buf = new ByteBuffer()
      buf.putVI(10)
      buf.putBytes(new TextEncoder().encode('short'))

      const frozen = buf.freeze()
      expect(() => TupleField.deserialize(frozen)).toThrow(NotEnoughBytesError)
    })
  })

  describe('Tuple', () => {
    it('should add, get, and convert to path', () => {
      const tuple = new Tuple()
      tuple.add(TupleField.fromUtf8('hello'))
      tuple.add(TupleField.fromUtf8('world'))

      expect(tuple.fields.length).toBe(2)
      expect(tuple.get(0).toUtf8()).toBe('hello')
      expect(tuple.get(1).toUtf8()).toBe('world')
      expect(tuple.toUtf8Path()).toBe('/hello/world')
    })

    it('should serialize and deserialize correctly', () => {
      const tuple = Tuple.fromUtf8Path('/hello/world')
      const serialized = tuple.serialize()

      const decoded = Tuple.deserialize(serialized)

      expect(tuple.equals(decoded)).toBe(true)
      expect(serialized.remaining).toBe(0)
    })

    it('should throw on too few bytes for a field', () => {
      const buf = new ByteBuffer()
      buf.putVI(2) // tuple with 2 fields
      buf.putVI(5)
      buf.putBytes(new TextEncoder().encode('hello'))
      buf.putVI(5) // but second field is incomplete

      const frozen = buf.freeze()
      expect(() => Tuple.deserialize(frozen)).toThrow(NotEnoughBytesError)
    })

    it('should support equality comparison', () => {
      const t1 = Tuple.fromUtf8Path('/hello/world')
      const t2 = Tuple.fromUtf8Path('/hello/world')
      const t3 = Tuple.fromUtf8Path('/hello/there')

      expect(t1.equals(t2)).toBe(true)
      expect(t1.equals(t3)).toBe(false)
    })

    it('should build from utf8 path correctly', () => {
      const tuple = Tuple.fromUtf8Path('/this/is/a/very/long/path')
      expect(tuple.fields.length).toBe(6)
      expect(tuple.toUtf8Path()).toBe('/this/is/a/very/long/path')
    })
  })
}
