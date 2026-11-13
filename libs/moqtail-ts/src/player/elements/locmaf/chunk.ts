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
 * Decodes LOCMAF Objects into a chunk's effective values (draft-einarsson-moq-locmaf-01 §15.1),
 * keeping the in-group reference that delta headers apply to (§12).
 */

import type { Location } from '@/model/common/location'
import type { TrackDefaults } from './init'
import { Field, LocmafError, parseObject, readList, unzigzag, type Fields } from './object'

/** A chunk's effective values (§15.1). */
export interface Chunk {
  baseMediaDecodeTime: bigint
  sampleDescriptionIndex: number
  durations: number[]
  sizes: number[]
  flags: number[]
  compositionTimeOffsets: number[]
  /** CENC auxiliary information; no IVs when the per-sample IV size is 0. */
  perSampleIVSize: number
  ivs: Uint8Array
  subsamples?: { counts: number[]; clear: number[]; protected: number[] }
  genBoxes: Uint8Array[]
  mdat: Uint8Array
}

/** Represented field values (§12.1): scalars, lists (signed for ID 5), and the raw IVs. */
type Represented = Map<number, bigint | bigint[] | Uint8Array>

/** No real chunk comes close; it bounds what a header can make the receiver allocate (§18). */
const MAX_SAMPLES = 1n << 24n
const SCALARS = [
  Field.sampleDescriptionIndex,
  Field.defaultSampleDuration,
  Field.defaultSampleSize,
  Field.defaultSampleFlags,
  Field.firstSampleFlags,
  Field.perSampleIVSize,
]
const CENC = [
  Field.initializationVector,
  Field.subsampleCount,
  Field.bytesOfClearData,
  Field.bytesOfProtectedData,
  Field.perSampleIVSize,
]

/** The in-group reference of one track: the previous chunk of its group, when it can be built on. */
export class Reference {
  #previous: { location: Location; fields?: Represented; nextBaseMediaDecodeTime?: bigint } | undefined

  /**
   * The effective values of the object at `location`, or the boxes of a rawBoxes object.
   * @throws LocmafError when it's malformed, or a delta with nothing to build on (§3, §9.3).
   */
  decode(location: Location, payload: Uint8Array, defaults: TrackDefaults): Chunk | Uint8Array {
    const previous = this.#previous
    // A gap in object IDs, a new group, a rawBoxes object or a rejected one leave no reference (§3)
    const reference =
      previous?.location.group === location.group && previous.location.object + 1n === location.object
        ? previous
        : undefined
    this.#previous = { location }
    const object = parseObject(payload)
    if ('raw' in object) return object.raw
    if (!object.full && !reference?.fields) throw new LocmafError('A delta header has no in-group reference')
    const fields = object.full ? absolute(object.fields) : applyDelta(object.fields, reference!.fields!)
    const baseMediaDecodeTime = object.full
      ? (fields.get(Field.baseMediaDecodeTime) as bigint)
      : reference!.nextBaseMediaDecodeTime!
    if (baseMediaDecodeTime >= 2n ** 64n) throw new LocmafError('The decode time overflows 64 bits')
    const chunk = effectiveValues(fields, defaults, { ...object, baseMediaDecodeTime })
    // A delta's BMDT is the previous one plus the previous chunk's durations (§12.2)
    const next = baseMediaDecodeTime + chunk.durations.reduce((sum, d) => sum + BigInt(d), 0n)
    this.#previous = { location, fields, nextBaseMediaDecodeTime: next }
    return chunk
  }
}

/** A full header's represented values (§11): absolute, except the zigzag ID 5. */
function absolute(fields: Fields): Represented {
  if (!fields.has(Field.sampleCount) || !fields.has(Field.baseMediaDecodeTime)) {
    throw new LocmafError('A full header needs trunSampleCount and tfdtBaseMediaDecodeTime')
  }
  const represented: Represented = new Map()
  for (const [id, value] of fields) {
    if (typeof value === 'bigint' || id === Field.initializationVector) represented.set(id, value)
    else represented.set(id, id === Field.compositionTimeOffsets ? readList(value).map(unzigzag) : readList(value))
  }
  return represented
}

/** The reference with a delta header applied: deletions, then scalar and list deltas (§12). */
function applyDelta(fields: Fields, reference: Represented): Represented {
  const next: Represented = new Map(reference)
  const deleted = fields.get(Field.deleted)
  if (deleted instanceof Uint8Array) for (const id of readList(deleted)) next.delete(Number(id))
  // The sample count first: it sets the list lengths
  for (const id of [Field.sampleCount, ...SCALARS]) {
    const delta = fields.get(id)
    if (delta === undefined) continue
    const value = ((next.get(id) as bigint | undefined) ?? 0n) + unzigzag(delta as bigint)
    if (value < 0n) throw new LocmafError(`Field ${id} goes negative`)
    next.set(id, value)
  }
  const n = sampleCount(next)
  for (const id of [Field.sampleDurations, Field.sampleFlags, Field.subsampleCount]) {
    applyList(next, fields, id, n)
  }
  applyList(next, fields, Field.sampleSizes, n === 0 ? 0 : n - 1)
  const subsamples = ((next.get(Field.subsampleCount) as bigint[] | undefined) ?? []).reduce((a, b) => a + b, 0n)
  if (subsamples > MAX_SAMPLES) throw new LocmafError(`${subsamples} subsamples is too many`)
  applyList(next, fields, Field.bytesOfClearData, Number(subsamples))
  applyList(next, fields, Field.bytesOfProtectedData, Number(subsamples))
  applyList(next, fields, Field.compositionTimeOffsets, n)
  const ivs = fields.get(Field.initializationVector)
  if (ivs) next.set(Field.initializationVector, ivs)
  return next
}

/**
 * Element-wise deltas against the previous list, absolute past its end (§12.1.1). An inherited
 * list follows a shorter count by truncation; one too short for a longer count is malformed.
 */
function applyList(next: Represented, fields: Fields, id: number, length: number): void {
  const previous = next.get(id) as bigint[] | undefined
  const bytes = fields.get(id) as Uint8Array | undefined
  if (!bytes) {
    if (previous && previous.length < length) throw new LocmafError(`Inherited field ${id} is too short`)
    if (previous) next.set(id, previous.slice(0, length))
    return
  }
  const deltas = readList(bytes).map(unzigzag)
  if (deltas.length !== length) throw new LocmafError(`Field ${id} has ${deltas.length} entries, not ${length}`)
  const list = deltas.map((delta, i) => (previous?.[i] ?? 0n) + delta)
  if (id !== Field.compositionTimeOffsets && list.some((v) => v < 0n))
    throw new LocmafError(`Field ${id} goes negative`)
  next.set(id, list)
}

/** The effective values from the represented fields, the CMAF Header and the object (§11.2, §15.1). */
function effectiveValues(
  fields: Represented,
  defaults: TrackDefaults,
  object: { genBoxes: Uint8Array[]; mdat: Uint8Array; baseMediaDecodeTime: bigint },
): Chunk {
  const n = sampleCount(fields)
  const scalar = (id: number, fallback: number) => {
    const value = fields.get(id) as bigint | undefined
    return value === undefined ? fallback : uint(value, 32)
  }
  const list = (id: number, length: number) => {
    const values = fields.get(id) as bigint[] | undefined
    if (values && values.length !== length)
      throw new LocmafError(`Field ${id} has ${values.length} entries, not ${length}`)
    return values
  }
  const durations = list(Field.sampleDurations, n)?.map((d) => uint(d, 32))
  const flags = list(Field.sampleFlags, n)?.map((f) => uint(f, 32))
  const firstFlags = fields.has(Field.firstSampleFlags) ? scalar(Field.firstSampleFlags, 0) : undefined
  const defaultFlags = scalar(Field.defaultSampleFlags, defaults.sampleFlags)
  const ctos = list(Field.compositionTimeOffsets, n)?.map((c) => {
    if (c < -(2n ** 31n) || c >= 2n ** 31n) throw new LocmafError(`Composition time offset ${c} is out of range`)
    return Number(c)
  })
  const sizes = sampleSizes(fields, n, object.mdat.length, defaults)
  return {
    baseMediaDecodeTime: object.baseMediaDecodeTime,
    sampleDescriptionIndex: scalar(Field.sampleDescriptionIndex, defaults.sampleDescriptionIndex),
    durations: durations ?? Array<number>(n).fill(scalar(Field.defaultSampleDuration, defaults.sampleDuration)),
    sizes,
    flags: flags ?? Array.from({ length: n }, (_, i) => (i === 0 ? (firstFlags ?? defaultFlags) : defaultFlags)),
    compositionTimeOffsets: ctos ?? Array<number>(n).fill(0),
    ...cenc(fields, defaults, sizes),
    genBoxes: object.genBoxes,
    mdat: object.mdat,
  }
}

/** Sample sizes (§11.2): the listed n − 1 and the rest of the mdat, a default, or the mdat for one sample. */
function sampleSizes(fields: Represented, n: number, payload: number, defaults: TrackDefaults): number[] {
  if (n === 0 && payload > 0) throw new LocmafError('A chunk without samples has an mdat payload')
  // The mdat box needs a 32-bit size (§15.7)
  if (payload > 0xfffffff7) throw new LocmafError('The mdat payload is too large')
  const listed = fields.get(Field.sampleSizes) as bigint[] | undefined
  if (listed) {
    if (listed.length !== Math.max(n - 1, 0)) throw new LocmafError(`${listed.length} sample sizes for ${n} samples`)
    const sizes = listed.map((s) => uint(s, 32))
    const sum = sizes.reduce((a, b) => a + b, 0)
    if (sum > payload) throw new LocmafError('The sample sizes exceed the mdat payload')
    return n === 0 ? [] : [...sizes, payload - sum]
  }
  const tfhd = fields.get(Field.defaultSampleSize) as bigint | undefined
  // A single sample is the whole payload, whatever trex says
  const size =
    tfhd !== undefined ? uint(tfhd, 32) : n !== 1 && defaults.sampleSize !== 0 ? defaults.sampleSize : undefined
  if (size !== undefined) {
    if (n * size !== payload) throw new LocmafError(`${n} samples of ${size} bytes don't fill ${payload}`)
    return Array<number>(n).fill(size)
  }
  if (n === 1) return [payload]
  if (payload === 0) return Array<number>(n).fill(0)
  throw new LocmafError(`No sample sizes for ${n} samples`)
}

/** CENC auxiliary information (§10.3, §13, §18), checked against the samples it describes. */
function cenc(
  fields: Represented,
  defaults: TrackDefaults,
  sizes: number[],
): Pick<Chunk, 'perSampleIVSize' | 'ivs' | 'subsamples'> {
  if (!defaults.isProtected && CENC.some((id) => fields.has(id))) {
    throw new LocmafError('CENC fields on a track that isn’t protected')
  }
  const n = sizes.length
  const ivSize = fields.has(Field.perSampleIVSize)
    ? uint(fields.get(Field.perSampleIVSize) as bigint, 8)
    : defaults.perSampleIVSize
  const ivs = (fields.get(Field.initializationVector) as Uint8Array | undefined) ?? new Uint8Array()
  if (ivs.length > 0 ? ivs.length !== ivSize * n : ivSize > 0 && n > 0) {
    throw new LocmafError(`${ivs.length} bytes of IVs for ${n} samples of ${ivSize}`)
  }
  const counts = (fields.get(Field.subsampleCount) as bigint[] | undefined)?.map((c) => uint(c, 16))
  const clear = (fields.get(Field.bytesOfClearData) as bigint[] | undefined)?.map((c) => uint(c, 16)) ?? []
  const protectedBytes = (fields.get(Field.bytesOfProtectedData) as bigint[] | undefined)?.map((p) => uint(p, 32)) ?? []
  if (!counts) {
    if (clear.length > 0 || protectedBytes.length > 0) throw new LocmafError('Subsample bytes without counts')
    return { perSampleIVSize: ivSize, ivs }
  }
  const total = counts.reduce((a, b) => a + b, 0)
  if (counts.length !== n || clear.length !== total || protectedBytes.length !== total) {
    throw new LocmafError('The subsample map doesn’t match the samples')
  }
  let at = 0
  counts.forEach((count, i) => {
    let covered = 0
    for (const end = at + count; at < end; at++) covered += clear[at]! + protectedBytes[at]!
    // A sample without subsamples carries no map
    if (count > 0 && covered !== sizes[i]) throw new LocmafError(`Subsamples cover ${covered} of ${sizes[i]} bytes`)
  })
  return { perSampleIVSize: ivSize, ivs, subsamples: { counts, clear, protected: protectedBytes } }
}

function sampleCount(fields: Represented): number {
  const n = fields.get(Field.sampleCount) as bigint | undefined
  if (n === undefined || n > MAX_SAMPLES) throw new LocmafError(`A sample count of ${n} can't be decoded`)
  return Number(n)
}

/** `value` as a number, when it fits an unsigned field of `bits`. */
function uint(value: bigint, bits: 8 | 16 | 32): number {
  if (value >= 1n << BigInt(bits)) throw new LocmafError(`${value} doesn’t fit ${bits} bits`)
  return Number(value)
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { ByteBuffer } = await import('@/model/common/byte_buffer')
  const { Location } = await import('@/model/common/location')
  const { locmafVectors } = await import('../../../../test/locmaf')
  const { trackDefaults } = await import('./init')

  const uniform = locmafVectors().find((v) => v.name === 'uniform')!
  const defaults = trackDefaults(uniform.init)
  const [full, delta, nextDelta] = uniform.objects.map((o) => o.payload)
  const at = (group: number, object: number) => new Location(group, object)
  const rejected = (reference: Reference, location: Location, payload: Uint8Array) =>
    expect(() => reference.decode(location, payload, defaults)).toThrow(LocmafError)

  test('a delta needs the previous object of its group: not after a gap, a new group, or a rejection', () => {
    const reference = new Reference()
    rejected(reference, at(0, 0), delta!)
    reference.decode(at(0, 1), full!, defaults)
    rejected(reference, at(0, 3), delta!)
    reference.decode(at(1, 0), full!, defaults)
    rejected(reference, at(2, 1), delta!)
    reference.decode(at(3, 0), full!, defaults)
    rejected(reference, at(3, 1), new Uint8Array([3, 1]))
    rejected(reference, at(3, 2), nextDelta!)
    // A full header re-anchors anywhere in a group
    reference.decode(at(3, 3), full!, defaults)
    expect(reference.decode(at(3, 4), delta!, defaults)).toMatchObject({ baseMediaDecodeTime: 4096n })
  })

  test('a rawBoxes object resets the reference', () => {
    const reference = new Reference()
    reference.decode(at(0, 0), full!, defaults)
    const free = new Uint8Array([0, 0, 0, 8, ...new TextEncoder().encode('free')])
    expect(reference.decode(at(0, 1), new Uint8Array([4, ...free]), defaults)).toEqual(free)
    rejected(reference, at(0, 2), delta!)
  })

  test('CENC fields on a track that isn’t protected are rejected', () => {
    const block = new ByteBuffer()
    for (const value of [14, 1, 10, 0, 16, 8]) block.putVI(value)
    const header = block.toUint8Array()
    rejected(new Reference(), at(0, 0), new Uint8Array([2, header.length, ...header, 1]))
  })
}
