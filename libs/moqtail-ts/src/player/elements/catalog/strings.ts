/**
 * Copyright 2026 The MOQtail Authors
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

/**
 * Catalog strings to MOQT bytes (MSF 11.2) and namespace splitting (MSF 5.2.2). Every
 * failure here makes the whole catalog invalid. Names compare as these bytes, never as text.
 */

import { Tuple, TupleField } from '@/model/common/tuple'
import { PlayerError } from '@/player/error'

const encoder = new TextEncoder()

/** The error for a catalog that can't be used at all. */
export function invalidCatalog(reason: string, cause?: unknown): PlayerError {
  return new PlayerError('UNSUPPORTED_SOURCE', `Invalid catalog: ${reason}`, { cause })
}

/**
 * The UTF-8 bytes of a track name, namespace field, or name reference (MSF 11.2.1).
 * @throws PlayerError for an unpaired surrogate or U+0000.
 */
export function toBytes(s: string): Uint8Array {
  if (!s.isWellFormed()) throw invalidCatalog(`${JSON.stringify(s)} has an unpaired surrogate (MSF 11.2.1)`)
  if (s.includes('\0')) throw invalidCatalog(`${JSON.stringify(s)} contains U+0000 (MSF 11.2.1)`)
  return encoder.encode(s)
}

/**
 * Splits a catalog namespace on `/` into 1 to 32 non-empty fields (MSF 5.2.2).
 * @throws PlayerError when it doesn't, or a field fails {@link toBytes}.
 */
export function splitNamespace(s: string): Tuple {
  const fields = s.split('/')
  if (fields.length > 32 || fields.includes('')) {
    throw invalidCatalog(`namespace ${JSON.stringify(s)} needs 1 to 32 non-empty fields (MSF 5.2.2)`)
  }
  return new Tuple(fields.map((field) => new TupleField(toBytes(field))))
}

/** A document starting with a UTF-8 byte order mark, which MSF 11.2.1 forbids. */
export const hasBom = (bytes: Uint8Array): boolean => bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const hex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join(' ')
  const fromJson = (json: string) => toBytes(JSON.parse(json) as string)

  describe('toBytes', () => {
    test('maps every row of MSF 11.2.5 Table 13', () => {
      expect(hex(fromJson('"video"'))).toBe('76 69 64 65 6f')
      expect(hex(fromJson('"café"'))).toBe('63 61 66 c3 a9')
      expect(hex(fromJson('"caf\\u00e9"'))).toBe('63 61 66 c3 a9')
      expect(hex(fromJson('"cafe\\u0301"'))).toBe('63 61 66 65 cc 81')
      expect(hex(fromJson('"Video"'))).toBe('56 69 64 65 6f')
      expect(fromJson('""')).toEqual(new Uint8Array())
      expect(() => fromJson('"\\uDEAD"')).toThrow(/unpaired surrogate/)
    })

    test('rejects U+0000', () => {
      expect(() => fromJson('"a\\u0000b"')).toThrow(/U\+0000/)
    })
  })

  describe('splitNamespace', () => {
    test('splits on every decoded solidus', () => {
      const namespace = splitNamespace(JSON.parse('"example.com\\/live\\u002Fchannel1"') as string)
      expect(namespace.fields.map((f) => f.toUtf8())).toEqual(['example.com', 'live', 'channel1'])
    })

    test('rejects empty fields and more than 32 fields', () => {
      for (const bad of ['', '/a', 'a/', 'a//b', Array(33).fill('x').join('/')]) {
        expect(() => splitNamespace(bad)).toThrow(PlayerError)
      }
      expect(splitNamespace(Array(32).fill('x').join('/')).fields).toHaveLength(32)
    })
  })

  test('hasBom', () => {
    expect(hasBom(new Uint8Array([0xef, 0xbb, 0xbf, 0x7b]))).toBe(true)
    expect(hasBom(encoder.encode('{}'))).toBe(false)
  })
}
