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
 * Just enough ISO-BMFF to place received data in media time: the `tfdt` base media decode
 * time of a fragment, and the `mdhd` timescale of an init segment. mp4box isn't needed for this.
 */

/** The payload of the first box at `path`, each name nested in the previous one. */
export function findBox(data: Uint8Array, path: readonly string[]): Uint8Array | undefined {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength)
  let [start, end] = [0, data.byteLength]
  for (const name of path) {
    let found = false
    while (start + 8 <= end) {
      let size = view.getUint32(start)
      let header = 8
      if (size === 1) {
        if (start + 16 > end) return undefined
        size = Number(view.getBigUint64(start + 8))
        header = 16
      } else if (size === 0) size = end - start
      if (size < header || start + size > end) return undefined
      const type = String.fromCharCode(...data.subarray(start + 4, start + 8))
      if (type === name) {
        ;[start, end] = [start + header, start + size]
        found = true
        break
      }
      start += size
    }
    if (!found) return undefined
  }
  return data.subarray(start, end)
}

/** `moof/traf/tfdt` base media decode time, in timescale units. */
export function baseMediaDecodeTime(fragment: Uint8Array): number | undefined {
  const tfdt = findBox(fragment, ['moof', 'traf', 'tfdt'])
  if (!tfdt || tfdt.length < 8) return undefined
  const view = new DataView(tfdt.buffer, tfdt.byteOffset, tfdt.byteLength)
  // A full box: version, then 3 bytes of flags.
  if (view.getUint8(0) === 1) return tfdt.length < 12 ? undefined : Number(view.getBigUint64(4))
  return view.getUint32(4)
}

/** The `moov/trak/mdia/mdhd` timescale of an init segment. */
export function mediaTimescale(init: Uint8Array): number | undefined {
  const mdhd = findBox(init, ['moov', 'trak', 'mdia', 'mdhd'])
  // Version 1 has 64-bit creation and modification times, version 0 32-bit ones.
  const offset = mdhd?.[0] === 1 ? 20 : 12
  if (!mdhd || mdhd.length < offset + 4) return undefined
  return new DataView(mdhd.buffer, mdhd.byteOffset, mdhd.byteLength).getUint32(offset)
}

/** Media time in seconds of a fragment; the catalog `timescale` wins over the init data's. */
export function fragmentTime(fragment: Uint8Array, timescale: number | undefined): number | undefined {
  const time = baseMediaDecodeTime(fragment)
  return time === undefined || !timescale ? undefined : time / timescale
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { box, fullBox, u32, u64 } = await import('@/player/testing/bmff')

  test('tfdt in versions 0 and 1, after other boxes, and nothing in other payloads', () => {
    const styp = box('styp', new Uint8Array(4))
    const v0 = [styp, box('moof', box('mfhd', new Uint8Array(8)), box('traf', fullBox('tfdt', 0, u32(90_000))))]
    const v1 = box('moof', box('traf', fullBox('tfdt', 1, u64(2n ** 33n))))
    expect(baseMediaDecodeTime(concat(...v0))).toBe(90_000)
    expect(baseMediaDecodeTime(v1)).toBe(2 ** 33)
    expect(baseMediaDecodeTime(new Uint8Array([1, 2]))).toBeUndefined()
    expect(baseMediaDecodeTime(box('mdat', new Uint8Array(16)))).toBeUndefined()
    expect(fragmentTime(concat(...v0), 90_000)).toBe(1)
  })

  test('mdhd timescale in versions 0 and 1', () => {
    const mdhd = (version: number) =>
      fullBox('mdhd', version, new Uint8Array(version === 1 ? 16 : 8), u32(48_000), new Uint8Array(8))
    for (const version of [0, 1]) {
      const init = concat(box('ftyp', new Uint8Array(8)), box('moov', box('trak', box('mdia', mdhd(version)))))
      expect(mediaTimescale(init)).toBe(48_000)
    }
  })

  const concat = (...parts: Uint8Array[]) => new Uint8Array(parts.flatMap((p) => [...p]))
}
