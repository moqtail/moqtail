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

import { findBox } from '@/player/elements/cmaf/bmff'
import { LocmafError } from './object'

/** What chunk reconstruction takes from the CMAF Header (draft-einarsson-moq-locmaf-01 §6). */
export interface TrackDefaults {
  trackId: number
  /** The `trex` defaults. */
  sampleDescriptionIndex: number
  sampleDuration: number
  sampleSize: number
  sampleFlags: number
  /** `tenc.default_isProtected`, and its per-sample IV size. */
  isProtected: boolean
  perSampleIVSize: number
}

/** Bytes before the child boxes of a protected visual and audio sample entry (ISO 14496-12 12.1.3, 12.2.3). */
const SAMPLE_ENTRY_FIELDS: Record<string, number> = { encv: 78, enca: 28 }

/** @throws LocmafError when `init` isn't a single-track CMAF Header with `trex`. */
export function trackDefaults(init: Uint8Array): TrackDefaults {
  const moov = findBox(init, ['moov'])
  if (!moov || countBoxes(moov, 'trak') !== 1) throw new LocmafError('The CMAF Header needs exactly one trak (§4.1)')
  const tkhd = view(findBox(moov, ['trak', 'tkhd']))
  const trex = view(findBox(moov, ['mvex', 'trex']))
  if (!tkhd || !trex || trex.byteLength < 24) throw new LocmafError('The CMAF Header has no tkhd or trex')
  const tenc = view(protection(moov))
  return {
    trackId: tkhd.getUint32(tkhd.getUint8(0) === 1 ? 20 : 12),
    sampleDescriptionIndex: trex.getUint32(8),
    sampleDuration: trex.getUint32(12),
    sampleSize: trex.getUint32(16),
    sampleFlags: trex.getUint32(20),
    isProtected: tenc !== undefined && tenc.byteLength >= 8 && tenc.getUint8(6) === 1,
    perSampleIVSize: tenc !== undefined && tenc.byteLength >= 8 ? tenc.getUint8(7) : 0,
  }
}

/** The `tenc` of the first sample entry, when it's a protected one. */
function protection(moov: Uint8Array): Uint8Array | undefined {
  const stsd = findBox(moov, ['trak', 'mdia', 'minf', 'stbl', 'stsd'])
  // A full box with an entry count, then the first entry
  const entry = stsd?.subarray(8)
  const skip =
    entry && entry.length >= 8 ? SAMPLE_ENTRY_FIELDS[String.fromCharCode(...entry.subarray(4, 8))] : undefined
  if (skip === undefined) return undefined
  return findBox(entry!.subarray(8 + skip), ['sinf', 'schi', 'tenc'])
}

function countBoxes(data: Uint8Array, type: string): number {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength)
  let count = 0
  for (let at = 0; at + 8 <= data.length; at += Math.max(view.getUint32(at), 8)) {
    if (String.fromCharCode(...data.subarray(at + 4, at + 8)) === type) count++
  }
  return count
}

function view(data: Uint8Array | undefined): DataView | undefined {
  return data && new DataView(data.buffer, data.byteOffset, data.byteLength)
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { box, fullBox, u32 } = await import('@/player/testing/bmff')

  const trex = fullBox('trex', 0, u32(1), u32(1), u32(1024), u32(200), u32(0x2000000))
  const tkhd = fullBox('tkhd', 1, new Uint8Array(16), u32(7))
  const tenc = fullBox('tenc', 0, new Uint8Array([0, 0, 1, 8]), new Uint8Array(16))
  const encv = box('encv', new Uint8Array(78), box('avcC'), box('sinf', box('frma'), box('schi', tenc)))
  const stsd = fullBox('stsd', 0, u32(1), encv)
  const trak = (...children: Uint8Array[]) => box('trak', tkhd, box('mdia', box('minf', box('stbl', ...children))))
  const init = (...traks: Uint8Array[]) => box('moov', ...traks, box('mvex', trex))

  test('track_ID, trex defaults and the tenc of a protected sample entry', () => {
    const defaults = { trackId: 7, sampleDescriptionIndex: 1, sampleDuration: 1024, sampleSize: 200 }
    expect(trackDefaults(init(trak(stsd)))).toEqual({
      ...defaults,
      sampleFlags: 0x2000000,
      isProtected: true,
      perSampleIVSize: 8,
    })
    expect(trackDefaults(init(trak()))).toMatchObject({ ...defaults, isProtected: false, perSampleIVSize: 0 })
  })

  test('a header with other than one trak, or without trex, is rejected', () => {
    expect(() => trackDefaults(init(trak(), trak()))).toThrow(LocmafError)
    expect(() => trackDefaults(box('moov', trak()))).toThrow(LocmafError)
  })
}
