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

import type { Chunk } from './chunk'
import type { TrackDefaults } from './init'
import { LocmafError } from './object'

/** tfhd and trun flags (ISO 14496-12 8.8.7, 8.8.8). */
const DEFAULT_BASE_IS_MOOF = 0x020000
const SAMPLE_DESCRIPTION_INDEX = 0x02
const DEFAULT_DURATION = 0x08
const DEFAULT_SIZE = 0x10
const DEFAULT_FLAGS = 0x20
const DATA_OFFSET = 0x001
const FIRST_SAMPLE_FLAGS = 0x004
const SAMPLE_DURATION = 0x100
const SAMPLE_SIZE = 0x200
const SAMPLE_FLAGS = 0x400
const SAMPLE_CTO = 0x800
/** moof header, mfhd and traf header, ahead of tfhd. */
const TRAF_START = 8 + 16 + 8

/**
 * The canonical CMAF chunk (draft-einarsson-moq-locmaf-01 §15): genBoxes, moof, mdat.
 * @throws LocmafError when a sample's CENC auxiliary information doesn't fit `saiz`.
 */
export function canonicalChunk(chunk: Chunk, defaults: TrackDefaults): Uint8Array {
  const n = chunk.sizes.length
  const tfhdDefault = (values: number[], trex: number) =>
    values.length > 0 && allEqual(values) && values[0] !== trex ? values[0] : undefined
  const durationsVary = !allEqual(chunk.durations)
  const sizesVary = !allEqual(chunk.sizes)
  // Equal flags after a different first one use first_sample_flags (§15.6)
  const firstFlags = n > 1 && !allEqual(chunk.flags) && allEqual(chunk.flags.slice(1))
  const flagsVary = !allEqual(chunk.flags) && !firstFlags
  const ctos = chunk.compositionTimeOffsets.some((c) => c !== 0)

  const optional = [
    [
      SAMPLE_DESCRIPTION_INDEX,
      chunk.sampleDescriptionIndex !== defaults.sampleDescriptionIndex ? chunk.sampleDescriptionIndex : undefined,
    ],
    [DEFAULT_DURATION, tfhdDefault(chunk.durations, defaults.sampleDuration)],
    [DEFAULT_SIZE, tfhdDefault(chunk.sizes, defaults.sampleSize)],
    [DEFAULT_FLAGS, flagsVary ? undefined : tfhdDefault(chunk.flags.slice(firstFlags ? 1 : 0), defaults.sampleFlags)],
  ].filter((entry): entry is [number, number] => entry[1] !== undefined)
  const tfhdFlags = optional.reduce((flags, [flag]) => flags | flag, DEFAULT_BASE_IS_MOOF)
  const tfhd = box('tfhd', u32(tfhdFlags, defaults.trackId, ...optional.map(([, value]) => value)))

  const tfdt = box('tfdt', u32(1 << 24), u64(chunk.baseMediaDecodeTime))

  const trFlags =
    DATA_OFFSET |
    (firstFlags ? FIRST_SAMPLE_FLAGS : 0) |
    (durationsVary ? SAMPLE_DURATION : 0) |
    (sizesVary ? SAMPLE_SIZE : 0) |
    (flagsVary ? SAMPLE_FLAGS : 0) |
    (ctos ? SAMPLE_CTO : 0)
  const version = chunk.compositionTimeOffsets.some((c) => c < 0) ? 1 : 0
  const records = chunk.sizes.flatMap((size, i) => [
    ...(durationsVary ? [chunk.durations[i]!] : []),
    ...(sizesVary ? [size] : []),
    ...(flagsVary ? [chunk.flags[i]!] : []),
    ...(ctos ? [chunk.compositionTimeOffsets[i]!] : []),
  ])
  // data_offset is set once the moof size is known
  const trun = box('trun', u32((version << 24) | trFlags, n, 0, ...(firstFlags ? [chunk.flags[0]!] : []), ...records))

  const traf = [tfhd, tfdt, trun, ...encryption(chunk, TRAF_START + tfhd.length + tfdt.length + trun.length)]
  const moof = box('moof', box('mfhd', u32(0, 0)), box('traf', ...traf))
  new DataView(moof.buffer).setUint32(TRAF_START + tfhd.length + tfdt.length + 16, moof.length + 8)
  return concat(...chunk.genBoxes, moof, u32(8 + chunk.mdat.length), type('mdat'), chunk.mdat)
}

/** saiz, saio and senc (§15.8), when the chunk has CENC auxiliary information; `at` is where saiz starts in the moof. */
function encryption(chunk: Chunk, at: number): Uint8Array[] {
  const { perSampleIVSize: ivSize, subsamples } = chunk
  if (ivSize === 0 && !subsamples) return []
  const n = chunk.sizes.length
  const auxSizes = chunk.sizes.map((_, i) => ivSize + (subsamples ? 2 + 6 * subsamples.counts[i]! : 0))
  if (auxSizes.some((size) => size > 255)) throw new LocmafError('CENC auxiliary information exceeds saiz')
  const uniform = allEqual(auxSizes)
  const saiz = box(
    'saiz',
    u32(0),
    new Uint8Array([uniform ? (auxSizes[0] ?? 0) : 0]),
    u32(n),
    ...(uniform ? [] : [new Uint8Array(auxSizes)]),
  )
  // senc follows saiz and the 20-byte saio; skip its header, version and flags, and sample_count
  const saio = box('saio', u32(0, 1, at + saiz.length + 20 + 16))
  const entries: Uint8Array[] = []
  let subsample = 0
  for (let i = 0; i < n; i++) {
    entries.push(chunk.ivs.subarray(i * ivSize, (i + 1) * ivSize))
    if (!subsamples) continue
    const count = subsamples.counts[i]!
    const map = new DataView(new ArrayBuffer(2 + 6 * count))
    map.setUint16(0, count)
    for (let j = 0; j < count; j++, subsample++) {
      map.setUint16(2 + 6 * j, subsamples.clear[subsample]!)
      map.setUint32(4 + 6 * j, subsamples.protected[subsample]!)
    }
    entries.push(new Uint8Array(map.buffer))
  }
  return [saiz, saio, box('senc', u32(subsamples ? 2 : 0, n), ...entries)]
}

function allEqual(list: number[]): boolean {
  return list.every((v) => v === list[0])
}

function box(name: string, ...parts: Uint8Array[]): Uint8Array {
  return concat(u32(8 + parts.reduce((size, part) => size + part.length, 0)), type(name), ...parts)
}

/** Big-endian 32-bit words; a negative one is written in two's complement. */
function u32(...words: number[]): Uint8Array {
  const out = new Uint8Array(4 * words.length)
  const view = new DataView(out.buffer)
  words.forEach((word, i) => view.setUint32(4 * i, word >>> 0))
  return out
}

function u64(value: bigint): Uint8Array {
  const out = new Uint8Array(8)
  new DataView(out.buffer).setBigUint64(0, value)
  return out
}

function type(name: string): Uint8Array {
  return new TextEncoder().encode(name)
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((size, part) => size + part.length, 0))
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const { findBox } = await import('@/player/elements/cmaf/bmff')
  const { locmafVectors } = await import('../../../../test/locmaf')
  const { Reference } = await import('./chunk')
  const { trackDefaults } = await import('./init')

  // The seqnum-* vectors carry mfhd sequence numbers in fields IDs 18 and 20, defined after -01
  const withoutSequenceNumber = (canonical: Uint8Array) => {
    const copy = canonical.slice()
    findBox(copy, ['moof', 'mfhd'])?.fill(0, 4, 8)
    return copy
  }

  test('every golden vector decodes to its canonical chunk', () => {
    const vectors = locmafVectors()
    expect(vectors).toHaveLength(18)
    for (const vector of vectors) {
      const defaults = trackDefaults(vector.init)
      const reference = new Reference()
      for (const { group, object, payload, canonical } of vector.objects) {
        const chunk = reference.decode(new Location(group, object), payload, defaults)
        const bytes = chunk instanceof Uint8Array ? chunk : canonicalChunk(chunk, defaults)
        expect({ vector: vector.name, group, object, bytes }).toEqual({
          vector: vector.name,
          group,
          object,
          bytes: withoutSequenceNumber(canonical),
        })
      }
    }
  })
}
