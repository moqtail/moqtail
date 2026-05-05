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

/** Returns true when `trackAlias` is a valid (non-negative) track alias. Zero is a legal value. */
export function isValidTrackAlias(trackAlias: bigint | undefined): trackAlias is bigint {
  return trackAlias !== undefined && trackAlias >= 0n
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  describe('isValidTrackAlias', () => {
    it('accepts zero', () => {
      expect(isValidTrackAlias(0n)).toBe(true)
    })
    it('accepts positive alias', () => {
      expect(isValidTrackAlias(1n)).toBe(true)
      expect(isValidTrackAlias(9999999999n)).toBe(true)
    })
    it('rejects undefined', () => {
      expect(isValidTrackAlias(undefined)).toBe(false)
    })
    it('rejects negative alias', () => {
      expect(isValidTrackAlias(-1n)).toBe(false)
    })
  })
}
