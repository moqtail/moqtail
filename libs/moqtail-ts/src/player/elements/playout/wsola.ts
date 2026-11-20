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
  const span = segment + overlap + seek
  // Coarse then fine, as SoundTouch's quick seek: about a seventh of a full search's correlations
  const coarse = 8
  // Reused across segments: this runs on the audio thread
  let output: Float32Array[] = []
  let tail: Float32Array[] = []
  let left = 0
  /** The last segment's `tail` is where it would have gone on, until a segment lands back on the play position. */
  let aligned = false

  /** Normalised cross-correlation of `from` with `plane` at `offset`. */
  function score(from: Float32Array, plane: Float32Array, offset: number): number {
    let product = 0
    let energy = 1e-9
    for (let i = 0; i < overlap; i++) {
      const sample = plane[offset + i]!
      product += from[i]! * sample
      energy += sample * sample
    }
    return product / Math.sqrt(energy)
  }

  /** The best of the offsets from `start` to `end`, `step` apart. */
  function scan(from: Float32Array, plane: Float32Array, start: number, end: number, step: number): number {
    let best = start
    let bestScore = -Infinity
    for (let offset = Math.max(0, start); offset <= Math.min(seek, end); offset += step) {
      const value = score(from, plane, offset)
      if (value > bestScore) {
        best = offset
        bestScore = value
      }
    }
    return best
  }

  function bestOffset(from: Float32Array, plane: Float32Array): number {
    const around = scan(from, plane, 0, seek, coarse)
    return scan(from, plane, around - coarse + 1, around + coarse - 1, 1)
  }

  return {
    /** Input samples `fill` reads from the play position on. */
    span,

    /** Samples queued from the play position for a segment at `rate`, which plays `rate` times its length. */
    needs(rate: number): number {
      return Math.max(span, Math.ceil(rate * segment))
    },

    /** At rate 1 it winds down with one segment that lands back on the play position. */
    running(rate: number): boolean {
      return rate !== 1 || aligned || left > 0
    },

    due(): boolean {
      return left === 0
    },

    /**
     * The next segment from `input`, `span` samples per channel from the play position. Returns
     * the samples the play position skips: at rate 1, the last segment's alignment.
     */
    fill(input: Float32Array[], rate: number): number {
      if (output.length !== input.length) {
        output = input.map(() => new Float32Array(segment))
        tail = input.map(() => new Float32Array(overlap))
        aligned = false
      }
      const offset = aligned ? bestOffset(tail[0]!, input[0]!) : 0
      for (const [channel, plane] of input.entries()) {
        const out = output[channel]!
        out.set(plane.subarray(offset, offset + segment))
        const from = tail[channel]!
        if (aligned) for (let i = 0; i < overlap; i++) out[i] = from[i]! + (out[i]! - from[i]!) * (i / overlap)
        from.set(plane.subarray(offset + segment, offset + segment + overlap))
      }
      aligned = rate !== 1
      left = segment
      return rate === 1 ? offset : 0
    },

    write(target: Float32Array[], at: number): void {
      const index = segment - left
      for (let channel = 0; channel < target.length; channel++)
        target[channel]![at] = (output[channel] ?? output[0]!)[index]!
      left--
    },

    reset(): void {
      aligned = false
      left = 0
    },
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const RATE = 48_000

  test('a sine keeps its pitch at 0.8, 1.25 and 2', () => {
    const tone = 440
    const input = Float32Array.from({ length: RATE * 2 }, (_, i) => Math.sin((2 * Math.PI * tone * i) / RATE))
    for (const rate of [0.8, 1.25, 2]) {
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
