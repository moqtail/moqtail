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

/** The framing of a LOCMAF Object (draft-einarsson-moq-locmaf-01 §7–9): its elements and header fields. */

import { FrozenByteBuffer } from '@/model/common/byte_buffer'

/** A malformed object, or one that can't be decoded where it sits in its group. */
export class LocmafError extends Error {}

/** A header's fields: even IDs as their vi64, odd IDs as their bytes (§7.3). */
export type Fields = Map<number, bigint | Uint8Array>

/** A moof-carrying object (§7.1), its genBoxes already wrapped into ISO boxes (§8.3), or a rawBoxes one (§9). */
export type LocmafObject =
  { genBoxes: Uint8Array[]; full: boolean; fields: Fields; mdat: Uint8Array } | { raw: Uint8Array }

/** Header field IDs (§10, Table 4). */
export const Field = {
  sampleSizes: 1,
  sampleDescriptionIndex: 2,
  sampleDurations: 3,
  defaultSampleDuration: 4,
  compositionTimeOffsets: 5,
  defaultSampleSize: 6,
  sampleFlags: 7,
  defaultSampleFlags: 8,
  initializationVector: 9,
  baseMediaDecodeTime: 10,
  subsampleCount: 11,
  firstSampleFlags: 12,
  bytesOfClearData: 13,
  sampleCount: 14,
  bytesOfProtectedData: 15,
  perSampleIVSize: 16,
  deleted: 27,
} as const

const KNOWN = new Set<bigint>(Object.values(Field).map(BigInt))
const GEN_BOX = 1n
const FULL_HEADER = 2n
const DELTA_HEADER = 3n
const RAW_BOXES = 4n

/** @throws LocmafError when the object is malformed. */
export function parseObject(payload: Uint8Array): LocmafObject {
  const buf = new FrozenByteBuffer(payload)
  const genBoxes: Uint8Array[] = []
  try {
    for (;;) {
      if (buf.remaining === 0) throw new LocmafError('The object ends before its header')
      const type = buf.getVI()
      if (type === GEN_BOX) {
        genBoxes.push(genBox(buf))
      } else if (type === FULL_HEADER || type === DELTA_HEADER) {
        const full = type === FULL_HEADER
        const fields = properties(buf.getBytes(length(buf)), full)
        return { genBoxes, full, fields, mdat: payload.subarray(buf.offset) }
      } else if (type === RAW_BOXES && genBoxes.length === 0) {
        return { raw: rawBoxes(payload.subarray(buf.offset)) }
      } else {
        // Not self-delimiting, so it can't be skipped (§7.1)
        throw new LocmafError(`Element type ${type} isn't allowed here`)
      }
    }
  } catch (error) {
    if (error instanceof LocmafError) throw error
    throw new LocmafError('The object is truncated', { cause: error })
  }
}

/** The vi64s of a list field, in order (§7.3). */
export function readList(bytes: Uint8Array): bigint[] {
  const buf = new FrozenByteBuffer(bytes)
  const values: bigint[] = []
  try {
    while (buf.remaining > 0) values.push(buf.getVI())
  } catch (error) {
    throw new LocmafError('A list field is truncated', { cause: error })
  }
  return values
}

/** The signed value of a zigzag vi64 (§7.4). */
export function unzigzag(z: bigint): bigint {
  return z & 1n ? -(z + 1n) / 2n : z / 2n
}

/** `box_size | box_name | payload` as the ISO box `uint32(4 + box_size) | box_name | payload` (§8.1, §8.3). */
function genBox(buf: FrozenByteBuffer): Uint8Array {
  const size = buf.getVI()
  if (size < 4n || size > 0xfffffffbn) throw new LocmafError(`genBox size ${size} is out of range`)
  const body = buf.getBytes(Number(size))
  const box = new Uint8Array(4 + body.length)
  new DataView(box.buffer).setUint32(0, box.length)
  box.set(body, 4)
  return box
}

function length(buf: FrozenByteBuffer): number {
  const value = buf.getVI()
  if (value > BigInt(buf.remaining)) throw new LocmafError(`A length of ${value} runs past the object`)
  return Number(value)
}

/** The known fields of a property block; unknown ones are skipped by their parity (§7.3). */
function properties(block: Uint8Array, full: boolean): Fields {
  const buf = new FrozenByteBuffer(block)
  const seen = new Set<bigint>()
  const fields: Fields = new Map()
  while (buf.remaining > 0) {
    const id = buf.getVI()
    if (seen.has(id)) throw new LocmafError(`Field ${id} repeats`)
    seen.add(id)
    const value = id % 2n === 0n ? buf.getVI() : buf.getBytes(length(buf))
    if (KNOWN.has(id)) fields.set(Number(id), value)
  }
  if (full && fields.has(Field.deleted)) throw new LocmafError('A full header lists deletions')
  if (!full && fields.has(Field.baseMediaDecodeTime)) throw new LocmafError('A delta header carries the BMDT')
  return fields
}

/** Complete boxes, each with its actual 32-bit size, filling the rest of the object (§9.1, §18). */
function rawBoxes(boxes: Uint8Array): Uint8Array {
  if (boxes.length === 0) throw new LocmafError('A rawBoxes element is empty')
  const view = new DataView(boxes.buffer, boxes.byteOffset, boxes.byteLength)
  for (let at = 0; at < boxes.length;) {
    const size = at + 8 <= boxes.length ? view.getUint32(at) : 0
    if (size < 8 || at + size > boxes.length) throw new LocmafError('rawBoxes holds an incomplete box')
    at += size
  }
  return boxes
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { ByteBuffer } = await import('@/model/common/byte_buffer')
  const { box } = await import('@/player/testing/bmff')

  const encode = (...items: (number | bigint | Uint8Array)[]) => {
    const buf = new ByteBuffer()
    for (const item of items) {
      if (item instanceof Uint8Array) buf.putBytes(item)
      else buf.putVI(item)
    }
    return buf.toUint8Array()
  }
  const bytes = (...values: number[]) => new Uint8Array(values)
  const type = (name: string) => new TextEncoder().encode(name)
  const malformed = (payload: Uint8Array) => expect(() => parseObject(payload)).toThrow(LocmafError)

  test('genBoxes, then a header whose unknown fields are skipped by parity, then the mdat', () => {
    const block = encode(14, 1, 10, 9000, 18, 300, 19, 2, bytes(7, 7))
    const object = parseObject(encode(1, 8, type('prft'), bytes(1, 2, 3, 4), 2, block.length, block, bytes(9, 9)))
    expect(object).toEqual({
      genBoxes: [box('prft', bytes(1, 2, 3, 4))],
      full: true,
      fields: new Map([
        [14, 1n],
        [10, 9000n],
      ]),
      mdat: bytes(9, 9),
    })
    expect(parseObject(encode(3, 0))).toMatchObject({ full: false, fields: new Map(), mdat: bytes() })
  })

  test('rawBoxes is the whole object, its boxes complete', () => {
    const styp = box('styp', bytes(1, 2, 3, 4))
    expect(parseObject(encode(4, styp))).toEqual({ raw: styp })
    for (const raw of [bytes(), styp.subarray(0, 10), box('free').fill(0, 0, 4), box('free').fill(1, 3, 4)]) {
      malformed(encode(4, raw))
    }
    malformed(encode(1, 4, type('free'), 4, styp))
  })

  test('malformed: unknown elements, bad genBox sizes, repeats, misplaced fields, truncation', () => {
    malformed(encode(5, 0))
    malformed(encode(1, 4, type('free')))
    malformed(encode(1, 3, type('fre'), 3, 0))
    malformed(encode(1, 0xfffffffc, type('free'), 3, 0))
    malformed(encode(1, 9, type('free'), 3, 0))
    malformed(encode(2, 4, 14, 1, 14, 1))
    malformed(encode(2, 3, 27, 1, 12))
    malformed(encode(3, 2, 10, 0))
    malformed(encode(3, 3, 14))
    malformed(encode(2, 5, 1))
  })

  test('zigzag: 0↔0, -1↔1, 1↔2, -2↔3', () => {
    expect([0n, 1n, 2n, 3n, 2n ** 64n - 1n].map(unzigzag)).toEqual([0n, -1n, 1n, -2n, -(2n ** 63n)])
  })
}
