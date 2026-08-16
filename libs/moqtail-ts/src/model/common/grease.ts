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
 * GREASE reservations (§14). Values matching `0x7f * N + 0x9D` are reserved across
 * several registries (Setup Options, Properties, error codes) so peers can emit them
 * to exercise unknown-value handling. Receivers treat them like any other unknown
 * value: ignore, never fatal.
 */

/** Step between successive GREASE values. */
export const GREASE_STEP = 0x7fn
/** The smallest GREASE value (N = 0). */
export const GREASE_BASE = 0x9dn

/** The largest GREASE value that still fits a varint, i.e. `0x3fffffffffffffde`. */
const GREASE_MAX = 0x3fffffffffffffden

/**
 * @public
 * True if `value` is a reserved GREASE value, i.e. `0x7f * N + 0x9D` for some
 * non-negative `N`.
 */
export function isGrease(value: bigint | number): boolean {
  const v = BigInt(value)
  return v >= GREASE_BASE && (v - GREASE_BASE) % GREASE_STEP === 0n
}

/**
 * @public
 * The `N`-th GREASE value, `0x7f * N + 0x9D`, or undefined if it would exceed what a
 * varint can carry (the pattern is capped at `0x3fffffffffffffde`).
 */
export function greaseValue(n: bigint | number): bigint | undefined {
  const value = GREASE_STEP * BigInt(n) + GREASE_BASE
  return value > GREASE_MAX ? undefined : value
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('grease', () => {
    test('recognizes the documented grease values', () => {
      expect(isGrease(0x9d)).toBe(true)
      expect(isGrease(0x11c)).toBe(true)
      expect(isGrease(GREASE_MAX)).toBe(true)
      for (let n = 0; n < 1000; n++) {
        expect(isGrease(greaseValue(n)!), `N=${n}`).toBe(true)
      }
    })

    test('rejects non-grease values', () => {
      for (const v of [0, 1, 0x02, 0x9c, 0x9e, 0x11b, 0x11d, 0x4000]) {
        expect(isGrease(v), `${v.toString(16)} must not be grease`).toBe(false)
      }
    })

    test('greaseValue matches the formula', () => {
      expect(greaseValue(0)).toBe(0x9dn)
      expect(greaseValue(1)).toBe(0x11cn)
      // The largest grease value the spec lists, and the first one past it.
      expect(greaseValue(0x8102040810203fn)).toBe(GREASE_MAX)
      expect(greaseValue(0x81020408102040n)).toBeUndefined()
    })
  })
}
