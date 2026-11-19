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

export type Stretch = ReturnType<typeof createStretch>

/**
 * WSOLA time stretch: audio at a rate other than 1 keeps its pitch. Each segment cross-fades in
 * from where the previous one would have gone on, at the input offset that matches it best.
 * The worklet loads its source text, so it references nothing outside itself.
 */
export function createStretch(sampleRate: number) {
  // SoundTouch's defaults
  const segment = Math.round(sampleRate * 0.04)
  const overlap = Math.round(sampleRate * 0.008)
  const seek = Math.round(sampleRate * 0.015)
  let output: Float32Array[] = []
  let left = 0
  /** Where the last segment would have gone on, until a segment at rate 1 lands back on the position. */
  let tail: Float32Array[] | undefined

  /** The offset into `plane` that matches `from` best, by normalised cross-correlation. */
  function bestOffset(from: Float32Array, plane: Float32Array): number {
    let best = 0
    let bestScore = -Infinity
    for (let offset = 0; offset <= seek; offset++) {
      let product = 0
      let energy = 1e-9
      for (let i = 0; i < overlap; i++) {
        const sample = plane[offset + i]!
        product += from[i]! * sample
        energy += sample * sample
      }
      const score = product / Math.sqrt(energy)
      if (score > bestScore) {
        best = offset
        bestScore = score
      }
    }
    return best
  }

  return {
    /** Input samples `fill` needs from the play position on. */
    span: segment + overlap + seek,

    /** At rate 1 it winds down with one segment that ends back on the play position. */
    running(rate: number): boolean {
      return rate !== 1 || tail !== undefined || left > 0
    },

    due(): boolean {
      return left === 0
    },

    /** The next segment from `input`, `span` samples per channel from the play position. */
    fill(input: Float32Array[], rate: number): void {
      const offset = tail && rate !== 1 ? bestOffset(tail[0]!, input[0]!) : 0
      output = input.map((plane, channel) => {
        const out = plane.slice(offset, offset + segment)
        const from = tail?.[channel]
        if (from) for (let i = 0; i < overlap; i++) out[i] = from[i]! + (out[i]! - from[i]!) * (i / overlap)
        return out
      })
      tail = rate === 1 ? undefined : input.map((plane) => plane.slice(offset + segment, offset + segment + overlap))
      left = segment
    },

    write(target: Float32Array[], at: number): void {
      const index = segment - left
      for (let channel = 0; channel < target.length; channel++)
        target[channel]![at] = (output[channel] ?? output[0]!)[index]!
      left--
    },

    reset(): void {
      tail = undefined
      left = 0
    },
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const RATE = 48_000

  test('a sine keeps its pitch at 0.8 and 1.25', () => {
    const tone = 440
    const input = Float32Array.from({ length: RATE * 2 }, (_, i) => Math.sin((2 * Math.PI * tone * i) / RATE))
    for (const rate of [0.8, 1.25]) {
      const stretch = createStretch(RATE)
      const out = [new Float32Array(RATE)]
      let position = 0
      for (let at = 0; at < out[0]!.length; at++, position += rate) {
        if (stretch.due()) stretch.fill([input.subarray(Math.floor(position))], rate)
        stretch.write(out, at)
      }
      let crossings = 0
      for (let i = 1; i < out[0]!.length; i++) if (out[0]![i - 1]! < 0 && out[0]![i]! >= 0) crossings++
      expect(Math.abs(crossings - tone)).toBeLessThan(tone * 0.02)
    }
  })
}
