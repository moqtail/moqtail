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

import { NotEnoughBytesError, VarIntOverflowError, CastingError } from '../error/error'
import { KeyValuePair } from './pair'
import { Location } from './location'
import { Tuple } from './tuple'
import { ReasonPhrase } from './reason_phrase'
import { FullTrackName } from '../data'

const MAX_VARINT_VALUE = 2n ** 64n - 1n

export abstract class BaseByteBuffer {
  protected buf: Uint8Array
  protected view: DataView
  protected _offset = 0
  protected _checkpoint = 0

  constructor(buf: Uint8Array) {
    this.buf = buf
    this.view = new DataView(buf.buffer, buf.byteOffset, buf.length)
  }

  get offset(): number {
    return this._offset
  }
  abstract get length(): number
  get remaining(): number {
    return this.length - this._offset
  }

  /**
   * Save current read position for potential rollback
   */
  checkpoint(): void {
    this._checkpoint = this._offset
  }

  /**
   * Restore read position to last checkpoint
   */
  restore(): void {
    this._offset = this._checkpoint
  }

  toUint8Array(): Uint8Array {
    return this.buf.slice() // Use slice() to create a copy
  }

  getU8(): number {
    if (this.remaining < 1) throw new NotEnoughBytesError('getU8', 1, this.remaining)
    return this.view.getUint8(this._offset++)
  }

  getU16(): number {
    if (this.remaining < 2) throw new NotEnoughBytesError('getU16', 2, this.remaining)
    const v = this.view.getUint16(this._offset, false)
    this._offset += 2
    return v
  }

  getVI(): bigint {
    if (this.remaining < 1) throw new NotEnoughBytesError('getVI.first_byte', 1, this.remaining)

    const first = this.getU8()

    const leadingOnes = this.countLeadingOnes(first)
    const length = leadingOnes + 1
    const extra = length - 1

    if (this.remaining < extra) {
      this._offset--
      throw new NotEnoughBytesError('getVI.continuation', length, this.remaining + 1)
    }

    let result = this.firstByteValueBits(first, length)
    for (let i = 0; i < extra; i++) {
      result = (result << 8n) | BigInt(this.getU8())
    }
    return result
  }

  countLeadingOnes(byte: number): number {
    let count = 0
    let mask = 0x80 // 1000_0000
    while (mask !== 0 && (byte & mask) !== 0) {
      count++
      mask >>= 1
    }
    return count
  }

  firstByteValueBits(first: number, length: number): bigint {
    if (length === 9) {
      return 0n
    }
    const dataBitsInFirstByte = 8 - length
    const mask = (1 << dataBitsInFirstByte) - 1
    return BigInt(first & mask)
  }

  getNumberVI(): number {
    let big = this.getVI()
    if (big > Number.MAX_SAFE_INTEGER)
      throw new CastingError(
        'BaseByteBuffer.getNumberVI()',
        'bigint',
        'number',
        'bigint exceeds Number.MAX_SAFE_INTEGER',
      )
    return Number(big)
  }
  getBytes(len: number): Uint8Array {
    if (this.remaining < len) throw new NotEnoughBytesError('getBytes', len, this.remaining)
    const slice = this.buf.slice(this._offset, this._offset + len) // Use slice() to create a copy
    this._offset += len
    return slice
  }

  getLengthPrefixedBytes(): Uint8Array {
    const len = this.getNumberVI()
    if (this.length < this.offset + len)
      throw new NotEnoughBytesError('BaseByteBuffer.getLengthPrefixedBytes', len, this.length - this.offset)
    return this.getBytes(len)
  }

  getKeyValuePair(): KeyValuePair {
    return KeyValuePair.deserialize(this)
  }

  getReasonPhrase(): ReasonPhrase {
    return ReasonPhrase.deserialize(this)
  }

  getLocation(): Location {
    return Location.deserialize(this)
  }

  getTuple(): Tuple {
    return Tuple.deserialize(this)
  }

  getFullTrackName(): FullTrackName {
    return FullTrackName.deserialize(this)
  }
}

export class ByteBuffer extends BaseByteBuffer {
  private _length = 0

  constructor(initialSize = 128) {
    super(new Uint8Array(initialSize))
  }
  get length(): number {
    return this._length
  }
  /**
   * Clear all data and reset all positions
   */
  clear(): void {
    this._length = 0
    this._offset = 0
    this._checkpoint = 0
    // Reset view to point to the beginning of the buffer
    this.buf = new Uint8Array()
    this.view = new DataView(this.buf.buffer, this.buf.byteOffset, this.buf.length)
  }
  /**
   * Drop all data before current offset and reset positions
   * This is the key method for memory management - removes processed data
   */
  commit(): void {
    if (this._offset === 0) {
      return // Nothing to commit
    }
    if (this._offset >= this._length) {
      // All data has been read
      this.clear()
      return
    }
    // Move unread data to the beginning of the buffer
    this.buf.set(this.buf.subarray(this._offset, this._length), 0)
    this._length = this._length - this._offset
    this._offset = 0
    this._checkpoint = 0
    // Update view to reflect the new buffer state
    this.view = new DataView(this.buf.buffer, this.buf.byteOffset, this.buf.length)
  }

  private ensureCapacity(add: number): void {
    const need = this._length + add
    if (need <= this.buf.length) return

    // TODO: Critical figure out and fix why buf.length is 0?
    let newSize = this.buf.length * 2 + 1
    while (newSize < need) newSize *= 2

    const newBuf = new Uint8Array(newSize)
    newBuf.set(this.buf.subarray(0, this._length))
    this.buf = newBuf
    this.view = new DataView(this.buf.buffer)
  }

  // --------- WRITE OPERATIONS ---------

  putU8(v: number): void {
    if (v < 0 || v > 0xff) {
      throw new RangeError(`Value ${v} is out of range for a U8 (0-255).`)
    }
    this.ensureCapacity(1)
    this.view.setUint8(this._length++, v)
  }
  putU16(v: number): void {
    if (v < 0 || v > 0xffff) {
      throw new RangeError(`Value ${v} is out of range for a U16 (0-65535).`)
    }
    this.ensureCapacity(2)
    this.view.setUint16(this._length, v, false)
    this._length += 2
  }

  /**
  /**
  * Encode a MOQT draft-18 varint using the minimal length.
  * See section 1.4.1 https://datatracker.ietf.org/doc/draft-ietf-moq-transport/
  */
  putVI(v: bigint | number): void {
    const value = typeof v === 'number' ? BigInt(v) : v

    if (value < 0n) {
      throw new CastingError('putVI', typeof v, 'unsigned varint', 'negative values are not supported')
    }
    if (value > MAX_VARINT_VALUE) {
      throw new VarIntOverflowError('putVI', Number(value))
    }

    // Choose the minimal length: lengths 1-8 hold 7*length bits, else 9 bytes.
    let length = 9
    for (let l = 1; l <= 8; l++) {
      if (value < 1n << BigInt(7 * l)) {
        length = l
        break
      }
    }

    this.ensureCapacity(length)

    if (length === 9) {
      // First byte 0xFF (all leading ones), then the full 64-bit value.
      this.view.setUint8(this._length++, 0xff)
      for (let i = 7; i >= 0; i--) {
        this.view.setUint8(this._length++, Number((value >> BigInt(8 * i)) & 0xffn))
      }
      return
    }

    const bytes: number[] = []

    for (let i = length - 1; i >= 0; i--) {
      bytes.push(Number((value >> BigInt(8 * i)) & 0xffn))
    }

    const ones = length - 1
    if (ones > 0) {
      const prefix = (((1 << ones) - 1) << (8 - ones)) & 0xff
      bytes[0] = (bytes[0] ?? 0) | prefix
    }

    for (const b of bytes) {
      this.view.setUint8(this._length++, b)
    }
  }

  putBytes(src: Uint8Array): void {
    this.ensureCapacity(src.length)
    this.buf.set(src, this._length)
    this._length += src.length
  }

  putLengthPrefixedBytes(src: Uint8Array): void {
    this.putVI(src.length)
    this.putBytes(src)
  }

  putKeyValuePair(pair: KeyValuePair): void {
    const b = pair.serialize().toUint8Array()
    this.putBytes(b)
  }

  putReasonPhrase(reason: ReasonPhrase): void {
    const b = reason.serialize().toUint8Array()
    this.putBytes(b)
  }
  override toUint8Array(): Uint8Array {
    return this.buf.slice(0, this._length) // Use slice() to create a copy
  }
  freeze(): FrozenByteBuffer {
    const snap = this.buf.slice(0, this._length) // Use slice() to create a copy
    return new FrozenByteBuffer(snap)
  }

  putLocation(loc: Location): void {
    this.putVI(loc.group)
    this.putVI(loc.object)
  }

  putTuple(tuple: Tuple): void {
    const serialized = tuple.serialize()
    this.putBytes(serialized.toUint8Array())
  }

  putFullTrackName(fullTrackName: FullTrackName): void {
    const serialized = fullTrackName.serialize()
    this.putBytes(serialized.toUint8Array())
  }
}

export class FrozenByteBuffer extends BaseByteBuffer {
  constructor(buf: Uint8Array) {
    super(buf)
  }
  get length(): number {
    return this.buf.length
  }
}

if (import.meta.vitest) {
  const { describe, expect, test } = import.meta.vitest
  describe('ByteBuffer', () => {
    describe('full track name', () => {
      test('roundtrip successful', () => {
        const original = FullTrackName.tryNew('namespace', 'track')
        const buf = new ByteBuffer()
        buf.putFullTrackName(original)

        const frozen = buf.freeze()
        const roundtripped = frozen.getFullTrackName()

        expect(roundtripped).toEqual(original)
      })
      test('partial bytes error', () => {
        const original = FullTrackName.tryNew('ns', 'trk')
        const buf = new ByteBuffer()
        buf.putFullTrackName(original)
        const frozen = buf.freeze()
        const partial = frozen.toUint8Array().slice(0, frozen.length - 2)
        const partialBuf = new FrozenByteBuffer(partial)
        expect(() => partialBuf.getFullTrackName()).toThrow()
      })
      test('excess bytes successful', () => {
        const original = FullTrackName.tryNew('ns', 'trk')
        const buf = new ByteBuffer()
        buf.putFullTrackName(original)
        buf.putU8(42)
        buf.putU8(99)
        const frozen = buf.freeze()
        const roundtripped = frozen.getFullTrackName()
        expect(roundtripped).toEqual(original)
        expect(frozen.getU8()).toBe(42)
        expect(frozen.getU8()).toBe(99)
      })
    })

    describe('checkpoint and restore', () => {
      test('can checkpoint and restore read position', () => {
        const buf = new ByteBuffer()
        buf.putU8(10)
        buf.putU8(20)
        buf.putU8(30)

        const readBuf = buf.freeze()

        // Read first byte
        expect(readBuf.getU8()).toBe(10)
        expect(readBuf.offset).toBe(1)

        // Save checkpoint
        readBuf.checkpoint()

        // Read more data
        expect(readBuf.getU8()).toBe(20)
        expect(readBuf.getU8()).toBe(30)
        expect(readBuf.offset).toBe(3)

        // Restore to checkpoint
        readBuf.restore()
        expect(readBuf.offset).toBe(1)

        // Can read the same data again
        expect(readBuf.getU8()).toBe(20)
        expect(readBuf.getU8()).toBe(30)
      })

      test('multiple checkpoints work correctly', () => {
        const buf = new ByteBuffer()
        buf.putU16(0x1234)
        buf.putU16(0x5678)
        buf.putU16(0x9abc)

        const readBuf = buf.freeze()

        // First checkpoint
        readBuf.checkpoint()
        expect(readBuf.getU16()).toBe(0x1234)

        // Second checkpoint (overwrites first)
        readBuf.checkpoint()
        expect(readBuf.getU16()).toBe(0x5678)

        // Restore to second checkpoint
        readBuf.restore()
        expect(readBuf.offset).toBe(2)
        expect(readBuf.getU16()).toBe(0x5678)
      })

      test('checkpoint and restore with complex deserialization', () => {
        const buf = new ByteBuffer()
        buf.putVI(42)
        buf.putU16(1000)
        buf.putBytes(new Uint8Array([1, 2, 3, 4]))

        const readBuf = buf.freeze()
        readBuf.checkpoint()

        // Simulate successful deserialization
        const vi = readBuf.getVI()
        const u16 = readBuf.getU16()
        const bytes = readBuf.getBytes(4)

        expect(vi).toBe(42n)
        expect(u16).toBe(1000)
        expect(bytes).toEqual(new Uint8Array([1, 2, 3, 4]))

        // Simulate failed operation - restore and try again
        readBuf.restore()
        expect(readBuf.offset).toBe(0)

        // Read again
        expect(readBuf.getVI()).toBe(42n)
        expect(readBuf.getU16()).toBe(1000)
        expect(readBuf.getBytes(4)).toEqual(new Uint8Array([1, 2, 3, 4]))
      })
    })

    describe('memory management with commit', () => {
      test('commit drops processed data and resets positions', () => {
        const buf = new ByteBuffer()
        buf.putU8(10)
        buf.putU8(20)
        buf.putU8(30)
        buf.putU8(40)

        expect(buf.length).toBe(4)

        // Read first two bytes
        expect(buf.getU8()).toBe(10)
        expect(buf.getU8()).toBe(20)
        expect(buf.offset).toBe(2)
        expect(buf.remaining).toBe(2)

        // Commit - should drop first two bytes
        buf.commit()
        expect(buf.offset).toBe(0)
        expect(buf.length).toBe(2)
        expect(buf.remaining).toBe(2)

        // Should only be able to read remaining data
        expect(buf.getU8()).toBe(30)
        expect(buf.getU8()).toBe(40)
        expect(buf.remaining).toBe(0)
      })

      test('commit after reading all data clears buffer completely', () => {
        const buf = new ByteBuffer()
        buf.putU8(10)
        buf.putU8(20)

        // Read all data
        buf.getU8()
        buf.getU8()
        expect(buf.remaining).toBe(0)

        // Commit should clear everything
        buf.commit()
        expect(buf.length).toBe(0)
        expect(buf.offset).toBe(0)
        expect(buf.remaining).toBe(0)
      })

      test('commit with no reads does nothing', () => {
        const buf = new ByteBuffer()
        buf.putU8(10)
        buf.putU8(20)

        const originalLength = buf.length
        buf.commit()

        expect(buf.length).toBe(originalLength)
        expect(buf.offset).toBe(0)
        expect(buf.getU8()).toBe(10)
        expect(buf.getU8()).toBe(20)
      })

      test('commit resets checkpoint position', () => {
        const buf = new ByteBuffer()
        buf.putU8(10)
        buf.putU8(20)
        buf.putU8(30)

        // Read and checkpoint
        buf.getU8() // offset = 1
        buf.checkpoint()
        buf.getU8() // offset = 2

        // Commit - should reset checkpoint to 0
        buf.commit()
        expect(buf.offset).toBe(0) // Restore should go to 0 (not the old checkpoint position)
        buf.restore()
        expect(buf.offset).toBe(0)
      })
    })

    describe('safe buffer access', () => {
      test('toUint8Array returns original data without advancing offset', () => {
        const buf = new ByteBuffer()
        buf.putU8(10)
        buf.putU8(20)
        buf.putU8(30)

        const originalOffset = buf.offset
        const copy = buf.toUint8Array()

        expect(buf.offset).toBe(originalOffset)
        expect(copy).toEqual(new Uint8Array([10, 20, 30]))

        // Verify it's a copy by modifying it
        copy[0] = 99
        expect(buf.getU8()).toBe(10) // Original data unchanged
      })

      test('frozen buffer provides immutable access', () => {
        const buf = new ByteBuffer()
        buf.putU16(0x1234)
        buf.putU16(0x5678)

        const frozen = buf.freeze()
        const originalOffset = frozen.offset
        const copy = frozen.toUint8Array()

        expect(frozen.offset).toBe(originalOffset)
        // Little endian: 0x1234 = [0x34, 0x12], 0x5678 = [0x78, 0x56]
        expect(copy).toEqual(new Uint8Array([0x12, 0x34, 0x56, 0x78]))

        // Verify frozen buffer's internal state isn't affected by external modifications
        copy[0] = 99
        expect(frozen.getU8()).toBe(0x12) // Original data changed
      })
    })

    describe('varint encoding/decoding', () => {
      const toBytes = (hex: number[]) => new Uint8Array(hex)
      const TEST_VECTORS: [bigint, number[]][] = [
        [37n, [0x25]],
        [15293n, [0xbb, 0xbd]],
        [226442877n, [0xed, 0x7f, 0x3e, 0x7d]],
        [2893212287960n, [0xfa, 0xa1, 0xa0, 0xe4, 0x03, 0xd8]],
        [151288809941952n, [0xfc, 0x89, 0x98, 0xab, 0xc6, 0x6b, 0xc0]],
        [70423237261249041n, [0xfe, 0xfa, 0x31, 0x8f, 0xa8, 0xe3, 0xca, 0x11]],
        [18446744073709551615n, [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]],
      ]

      test('test vectors encode to exact bytes', () => {
        for (const [value, expected] of TEST_VECTORS) {
          const buf = new ByteBuffer()
          buf.putVI(value)
          expect(buf.toUint8Array()).toEqual(toBytes(expected))
        }
      })

      test('test vectors decode to exact values', () => {
        for (const [value, encoding] of TEST_VECTORS) {
          const buf = new FrozenByteBuffer(toBytes(encoding))
          expect(buf.getVI()).toBe(value)
          expect(buf.remaining).toBe(0)
        }
      })

      test('encodes minimal length at each boundary', () => {
        // (value, expected byte length), just below/above every length transition.
        const cases: [bigint, number][] = [
          [0n, 1],
          [127n, 1],
          [128n, 2],
          [16383n, 2],
          [16384n, 3],
          [2097151n, 3],
          [2097152n, 4],
          [268435455n, 4],
          [268435456n, 5],
          [34359738367n, 5],
          [34359738368n, 6],
          [4398046511103n, 6],
          [4398046511104n, 7],
          [562949953421311n, 7],
          [562949953421312n, 8],
          [72057594037927935n, 8],
          [72057594037927936n, 9],
          [18446744073709551615n, 9],
        ]
        for (const [value, len] of cases) {
          const buf = new ByteBuffer()
          buf.putVI(value)
          expect(buf.length).toBe(len)
        }
      })

      test('round-trips across all lengths (bigint and number)', () => {
        const values = [
          0n,
          1n,
          63n,
          64n,
          127n,
          128n,
          16383n,
          16384n,
          2097151n,
          2097152n,
          268435455n,
          268435456n,
          34359738367n,
          34359738368n,
          4398046511103n,
          4398046511104n,
          562949953421311n,
          562949953421312n,
          72057594037927935n,
          72057594037927936n,
          18446744073709551615n,
        ]
        for (const value of values) {
          const buf = new ByteBuffer()
          buf.putVI(value)
          const frozen = buf.freeze()
          expect(frozen.getVI()).toBe(value)
        }
        // number inputs should behave identically to their bigint equivalents.
        for (const value of [0, 63, 64, 127, 128, 16384]) {
          const buf = new ByteBuffer()
          buf.putVI(value)
          expect(buf.freeze().getVI()).toBe(BigInt(value))
        }
      })

      test('decodes non-minimal encodings', () => {
        // The spec allows non-minimal encodings; decoders must accept them.
        expect(new FrozenByteBuffer(toBytes([0x25])).getVI()).toBe(37n) // minimal
        expect(new FrozenByteBuffer(toBytes([0x80, 0x25])).getVI()).toBe(37n) // 2-byte
        // 0 padded out to the maximum 9-byte form.
        expect(new FrozenByteBuffer(toBytes([0xff, 0, 0, 0, 0, 0, 0, 0, 0])).getVI()).toBe(0n)
      })

      test('throws on negative values', () => {
        const buf = new ByteBuffer()
        expect(() => buf.putVI(-1)).toThrow()
      })

      test('throws on values too large to encode', () => {
        const buf = new ByteBuffer()
        const tooLarge = 18446744073709551616n // 2^64, one more than max
        expect(() => buf.putVI(tooLarge)).toThrow()
      })

      test('throws on empty buffer', () => {
        expect(() => new FrozenByteBuffer(new Uint8Array()).getVI()).toThrow()
      })

      test('throws on truncated continuation', () => {
        expect(() => new FrozenByteBuffer(toBytes([0x80])).getVI()).toThrow() // 2-byte, missing 1
        expect(() => new FrozenByteBuffer(toBytes([0xc0, 0x00])).getVI()).toThrow() // 3-byte, missing 1
        expect(() => new FrozenByteBuffer(toBytes([0xff])).getVI()).toThrow() // 9-byte, only prefix
      })

      test('leaves trailing bytes untouched', () => {
        const buf = new FrozenByteBuffer(toBytes([0x25, 0xaa, 0xbb]))
        expect(buf.getVI()).toBe(37n)
        expect(buf.getU8()).toBe(0xaa)
        expect(buf.getU8()).toBe(0xbb)
      })

      test('rolls back offset on truncated continuation', () => {
        const buf = new FrozenByteBuffer(toBytes([0x80]))
        expect(() => buf.getVI()).toThrow()
        expect(buf.offset).toBe(0)
      })
    })

    describe('basic operations', () => {
      test('u8 operations', () => {
        const buf = new ByteBuffer()
        buf.putU8(42)
        buf.putU8(255)
        const readBuf = buf.freeze()
        expect(readBuf.getU8()).toBe(42)
        expect(readBuf.getU8()).toBe(255)
        expect(readBuf.remaining).toBe(0)
      })

      test('u16 operations', () => {
        const buf = new ByteBuffer()
        buf.putU16(258) // 0x0102 (little endian: [0x02, 0x01])
        buf.putU16(65535) // 0xFFFF
        const readBuf = buf.freeze()
        expect(readBuf.getU16()).toBe(258)
        expect(readBuf.getU16()).toBe(65535)
        expect(readBuf.remaining).toBe(0)
      })

      test('bytes operations', () => {
        const buf = new ByteBuffer()
        const bytes = new Uint8Array([1, 2, 3, 4, 5])
        buf.putBytes(bytes)
        const readBuf = buf.freeze()
        const readBytes = readBuf.getBytes(5)
        expect(readBytes).toEqual(bytes)
        expect(readBuf.remaining).toBe(0)
      })

      test('length-prefixed bytes', () => {
        const buf = new ByteBuffer()
        const bytes = new Uint8Array([10, 20, 30, 40, 50])
        buf.putLengthPrefixedBytes(bytes)
        const readBuf = buf.freeze()
        const readBytes = readBuf.getLengthPrefixedBytes()
        expect(readBytes).toEqual(bytes)
        expect(readBuf.remaining).toBe(0)
      })
    })

    describe('buffer capacity', () => {
      test('grows automatically when needed', () => {
        const buf = new ByteBuffer(4)
        expect(buf.length).toBe(0)
        const bytes = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])
        buf.putBytes(bytes)
        expect(buf.length).toBe(8)
        const readBuf = buf.freeze()
        expect(readBuf.getBytes(8)).toEqual(bytes)
      })
    })

    describe('freeze', () => {
      test('creates immutable copy with same data', () => {
        const buf = new ByteBuffer()
        buf.putU8(1)
        buf.putU8(2)
        buf.putU8(3)
        const frozen = buf.freeze()
        expect(frozen.length).toBe(3)
        expect(frozen.offset).toBe(0)
        expect(frozen.getU8()).toBe(1)
        expect(frozen.getU8()).toBe(2)
        expect(frozen.getU8()).toBe(3)
        expect(frozen.remaining).toBe(0)
      })
    })

    describe('error handling', () => {
      test('throws not enough bytes error', () => {
        const buf = new ByteBuffer()
        buf.putU8(42)
        const readBuf = buf.freeze()
        readBuf.getU8()
        expect(() => readBuf.getU8()).toThrow('not enough bytes')
        expect(() => readBuf.getU16()).toThrow('not enough bytes')
        expect(() => readBuf.getBytes(1)).toThrow('not enough bytes')
      })
    })

    describe('reason phrase', () => {
      test('putReasonPhrase and getReasonPhrase roundtrip', () => {
        const phrase = new ReasonPhrase('test reason')
        const buf = new ByteBuffer()
        buf.putReasonPhrase(phrase)
        const frozen = buf.freeze()
        const readPhrase = frozen.getReasonPhrase()
        expect(readPhrase.phrase).toBe('test reason')
      })
    })

    describe('key value pair', () => {
      test('putKeyValuePair and getKeyValuePair roundtrip (varint) and matches serialize', () => {
        const pair = KeyValuePair.tryNewVarInt(2, 12345n)
        const buf = new ByteBuffer()
        buf.putKeyValuePair(pair)
        const frozen = buf.freeze()
        const readPair = frozen.getKeyValuePair()
        expect(readPair).toEqual(pair)
        // Assert that the serialized bytes match
        expect(frozen.toUint8Array()).toEqual(pair.serialize().toUint8Array())
      })

      test('putKeyValuePair and getKeyValuePair roundtrip (bytes) and matches serialize', () => {
        const data = new TextEncoder().encode('hello')
        const pair = KeyValuePair.tryNewBytes(1, data)
        const buf = new ByteBuffer()
        buf.putKeyValuePair(pair)
        const frozen = buf.freeze()
        const readPair = frozen.getKeyValuePair()
        expect(readPair).toEqual(pair)
        expect(frozen.toUint8Array()).toEqual(pair.serialize().toUint8Array())
      })
    })
  })
}
