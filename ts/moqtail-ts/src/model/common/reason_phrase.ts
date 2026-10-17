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

import { LengthExceedsMaxError, InvalidUTF8Error, NotEnoughBytesError, CastingError } from '../error/error'
import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from './byte_buffer'
/**
 * The maximum allowed length (in bytes) for a ReasonPhrase.
 * @public
 */
export const MAX_REASON_PHRASE_LEN = 1024

/**
 * Represents a protocol ReasonPhrase, a short UTF-8 string used for error or status reporting.
 * Enforces a maximum byte length and validates encoding.
 *
 * @public
 */
export class ReasonPhrase {
  /**
   * The underlying phrase string.
   * @public
   */
  readonly #phrase: string

  /**
   * Constructs a ReasonPhrase, validating UTF-8 encoding and length.
   *
   * @param phrase - The string to use as the reason phrase.
   * @throws {@link InvalidUTF8Error} if encoding fails.
   * @throws {@link LengthExceedsMaxError} if the encoded phrase exceeds {@link MAX_REASON_PHRASE_LEN} bytes.
   * @public
   */
  constructor(phrase: string) {
    let encodedPhrase: Uint8Array
    try {
      encodedPhrase = new TextEncoder().encode(phrase)
    } catch (e) {
      const errorMessage = e instanceof Error ? e.message : String(e)
      throw new InvalidUTF8Error('ReasonPhrase.constructor (TextEncoder failed)', errorMessage)
    }

    if (encodedPhrase.length > MAX_REASON_PHRASE_LEN) {
      throw new LengthExceedsMaxError('ReasonPhrase.constructor', MAX_REASON_PHRASE_LEN, encodedPhrase.length)
    }
    this.#phrase = phrase
  }

  /**
   * Returns the phrase string.
   * @public
   */
  public get phrase(): string {
    return this.#phrase
  }

  /**
   * Serializes the ReasonPhrase into a {@link FrozenByteBuffer} containing:
   * varint(length_of_phrase_bytes) || phrase_bytes
   *
   * @returns The serialized buffer.
   * @public
   */
  public serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    const phraseBytes = new TextEncoder().encode(this.#phrase)
    buf.putVI(phraseBytes.length)
    buf.putBytes(phraseBytes)
    return buf.freeze()
  }

  /**
   * Deserializes a ReasonPhrase from the given buffer.
   * Reads varint(length) || utf8‑bytes.
   *
   * @param buf - The buffer to read from.
   * @returns The deserialized ReasonPhrase.
   * @throws :{@link CastingError} if the length cannot be safely cast to a number.
   * @throws :{@link LengthExceedsMaxError} if the length exceeds {@link MAX_REASON_PHRASE_LEN}.
   * @throws :{@link NotEnoughBytesError} if the buffer does not contain enough bytes.
   * @throws :{@link InvalidUTF8Error} if decoding fails.
   * @public
   */
  public static deserialize(buf: BaseByteBuffer): ReasonPhrase {
    const lenBig = buf.getVI() // Read length varint
    let len: number
    try {
      len = Number(lenBig)
      if (BigInt(len) !== lenBig) {
        // This handles cases where lenBig is too large to be safely represented by a JS number,
        // e.g., > Number.MAX_SAFE_INTEGER. MAX_REASON_PHRASE_LEN (1024) is well within this.
        throw new Error(`Value ${lenBig.toString()} cannot be accurately represented as a number.`)
      }
    } catch (e) {
      const errorDetails = e instanceof Error ? e.message : String(e)
      throw new CastingError('ReasonPhrase.deserialize length', 'bigint', 'number', errorDetails)
    }

    if (len > MAX_REASON_PHRASE_LEN) {
      throw new LengthExceedsMaxError('ReasonPhrase.deserialize', MAX_REASON_PHRASE_LEN, len)
    }

    if (buf.remaining < len) {
      throw new NotEnoughBytesError('ReasonPhrase.deserialize value', len, buf.remaining)
    }
    const phraseBytes = buf.getBytes(len) // Read the actual phrase bytes

    try {
      const phraseStr = new TextDecoder('utf-8', { fatal: true }).decode(phraseBytes)
      return new ReasonPhrase(phraseStr)
    } catch (e) {
      const errorMessage = e instanceof Error ? e.message : String(e)
      throw new InvalidUTF8Error('ReasonPhrase.deserialize (decode)', errorMessage)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('ReasonPhrase', () => {
    test('constructor and getter', () => {
      const phraseStr = 'Hello World \u{1F30F}!' // Includes a 4-byte UTF-8 char
      const rp = new ReasonPhrase(phraseStr)
      expect(rp.phrase).toBe(phraseStr)
    })

    test('roundtrip', () => {
      const original = new ReasonPhrase('hello world')
      const serializedBytes = original.serialize().toUint8Array()

      const buf = new ByteBuffer()
      buf.putBytes(serializedBytes)
      // Freeze to get a FrozenByteBuffer, or use as is if ByteBuffer is accepted by deserialize
      const frozenBuf = buf.freeze()

      const deserialized = ReasonPhrase.deserialize(frozenBuf)
      expect(deserialized.phrase).toBe(original.phrase)
      expect(frozenBuf.remaining).toBe(0)
    })

    test('empty phrase roundtrip', () => {
      const original = new ReasonPhrase('')
      const serializedBytes = original.serialize().toUint8Array()

      const buf = new ByteBuffer()
      buf.putBytes(serializedBytes)
      const frozenBuf = buf.freeze()

      const deserialized = ReasonPhrase.deserialize(frozenBuf)
      expect(deserialized.phrase).toBe(original.phrase)
      expect(frozenBuf.remaining).toBe(0)
    })

    test('constructor throws for too long phrase (byte length)', () => {
      // Create a string whose UTF-8 byte representation is > MAX_REASON_PHRASE_LEN
      // A character like '€' is 3 bytes in UTF-8.
      // MAX_REASON_PHRASE_LEN = 1024. So, 1024/3 = 341.33. 342 '€' chars will be > 1024 bytes.
      const euro = '€' // 3 bytes in UTF-8
      const longPhrase = euro.repeat(Math.ceil((MAX_REASON_PHRASE_LEN + 1) / euro.length))
      expect(new TextEncoder().encode(longPhrase).length).toBeGreaterThan(MAX_REASON_PHRASE_LEN)
      expect(() => new ReasonPhrase(longPhrase)).toThrow(LengthExceedsMaxError)
    })

    test('roundtrip with unpaired high surrogate', () => {
      // Unpaired high surrogate (not valid Unicode, but JS allows it in strings)
      const invalidStr = 'test\uD800test'
      // Ensure TextEncoder encodes it as bytes
      const encoded = new TextEncoder().encode(invalidStr)
      const rp = new ReasonPhrase(invalidStr)
      const serialized = rp.serialize()
      const deserialized = ReasonPhrase.deserialize(serialized)
      expect(new TextEncoder().encode(deserialized.phrase)).toEqual(encoded)
    })

    test('roundtrip with unpaired low surrogate', () => {
      const invalidStr = 'test\uDC00test' // Unpaired low surrogate
      const encoded = new TextEncoder().encode(invalidStr)
      const rp = new ReasonPhrase(invalidStr)
      const serialized = rp.serialize()
      const deserialized = ReasonPhrase.deserialize(serialized)
      expect(new TextEncoder().encode(deserialized.phrase)).toEqual(encoded)
    })

    test('deserialize throws for length exceeding max', () => {
      const buf = new ByteBuffer()
      buf.putVI(MAX_REASON_PHRASE_LEN + 1) // Indicate length > max
      buf.putBytes(new Uint8Array(0)) // No actual data needed

      const frozenBuf = buf.freeze()
      expect(() => ReasonPhrase.deserialize(frozenBuf)).toThrow(LengthExceedsMaxError)
    })

    test('deserialize throws for insufficient bytes', () => {
      const buf = new ByteBuffer()
      buf.putVI(10) // Expect 10 bytes
      buf.putBytes(new TextEncoder().encode('short')) // Provide only 5 bytes

      const frozenBuf = buf.freeze()
      expect(() => ReasonPhrase.deserialize(frozenBuf)).toThrow(NotEnoughBytesError)
    })

    test('deserialize throws for invalid utf8 bytes', () => {
      const buf = new ByteBuffer()
      buf.putVI(2) // Expect 2 bytes
      // Invalid UTF-8 sequence (e.g., an isolated surrogate or an overlong encoding)
      // Using 0xFF, 0xFE which is a common example of invalid UTF-8 bytes
      buf.putBytes(new Uint8Array([0xff, 0xfe]))

      const frozenBuf = buf.freeze()
      expect(() => ReasonPhrase.deserialize(frozenBuf)).toThrow(InvalidUTF8Error)
    })

    test('deserialize handles max length phrase', () => {
      const phraseStr = 'a'.repeat(MAX_REASON_PHRASE_LEN)
      const original = new ReasonPhrase(phraseStr)
      const serializedBytes = original.serialize().toUint8Array()

      const buf = new ByteBuffer()
      buf.putBytes(serializedBytes)
      const frozenBuf = buf.freeze()

      const deserialized = ReasonPhrase.deserialize(frozenBuf)
      expect(deserialized.phrase).toBe(original.phrase)
      expect(frozenBuf.remaining).toBe(0)
    })

    test('deserialize length casting error for extremely large varint', () => {
      const buf = new ByteBuffer()
      // A bigint that will definitely not fit in a JS number safely
      const veryLargeLength = BigInt(Number.MAX_SAFE_INTEGER) + 100n
      buf.putVI(veryLargeLength)
      buf.putBytes(new Uint8Array(0)) // No data needed

      const frozenBuf = buf.freeze()
      // This should throw CastingError because Number(veryLargeLength) loses precision
      // or cannot represent the number, before it even gets to the MAX_REASON_PHRASE_LEN check.
      expect(() => ReasonPhrase.deserialize(frozenBuf)).toThrow(CastingError)
    })
  })
}
